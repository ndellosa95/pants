// Copyright 2023 Pants project contributors (see CONTRIBUTORS.md).
// Licensed under the Apache License, Version 2.0 (see LICENSE).
use std::convert::TryInto;
use std::sync::Arc;

use async_trait::async_trait;
use grpc_util::retry::{retry_call, status_is_retryable};
use grpc_util::{headers_to_http_header_map, layered_service, status_to_str, LayeredService};
use hashing::Digest;
use protos::gen::build::bazel::remote::execution::v2 as remexec;
use remexec::action_cache_client::ActionCacheClient;
use remexec::ActionResult;
use workunit_store::Metric;

use crate::remote::apply_headers;
use process_execution::Context;
use tonic::{Code, Request};

use super::{ActionCacheProvider, RemoteCacheProviderOptions};

pub struct Provider {
  instance_name: Option<String>,
  action_cache_client: Arc<ActionCacheClient<LayeredService>>,
}

impl Provider {
  pub fn new(
    RemoteCacheProviderOptions {
      instance_name,
      action_cache_address,
      root_ca_certs,
      mut headers,
      concurrency_limit,
      rpc_timeout,
    }: RemoteCacheProviderOptions,
  ) -> Result<Self, String> {
    let tls_client_config = if action_cache_address.starts_with("https://") {
      Some(grpc_util::tls::Config::new_without_mtls(root_ca_certs).try_into()?)
    } else {
      None
    };

    let endpoint = grpc_util::create_endpoint(
      &action_cache_address,
      tls_client_config.as_ref(),
      &mut headers,
    )?;
    let http_headers = headers_to_http_header_map(&headers)?;
    let channel = layered_service(
      tonic::transport::Channel::balance_list(vec![endpoint].into_iter()),
      concurrency_limit,
      http_headers,
      Some((rpc_timeout, Metric::RemoteCacheRequestTimeouts)),
    );
    let action_cache_client = Arc::new(ActionCacheClient::new(channel));

    Ok(Provider {
      instance_name,
      action_cache_client,
    })
  }
}

#[async_trait]
impl ActionCacheProvider for Provider {
  async fn update_action_result(
    &self,
    action_digest: Digest,
    action_result: ActionResult,
  ) -> Result<(), String> {
    let client = self.action_cache_client.as_ref().clone();
    retry_call(
      client,
      move |mut client| {
        let update_action_cache_request = remexec::UpdateActionResultRequest {
          instance_name: self.instance_name.clone().unwrap_or_else(|| "".to_owned()),
          action_digest: Some(action_digest.into()),
          action_result: Some(action_result.clone()),
          ..remexec::UpdateActionResultRequest::default()
        };

        async move {
          client
            .update_action_result(update_action_cache_request)
            .await
        }
      },
      status_is_retryable,
    )
    .await
    .map_err(status_to_str)?;

    Ok(())
  }

  async fn get_action_result(
    &self,
    action_digest: Digest,
    context: &Context,
  ) -> Result<Option<ActionResult>, String> {
    let client = self.action_cache_client.as_ref().clone();
    let response = retry_call(
      client,
      move |mut client| {
        let request = remexec::GetActionResultRequest {
          action_digest: Some(action_digest.into()),
          instance_name: self.instance_name.clone().unwrap_or_default(),
          ..remexec::GetActionResultRequest::default()
        };
        let request = apply_headers(Request::new(request), &context.build_id);
        async move { client.get_action_result(request).await }
      },
      status_is_retryable,
    )
    .await;

    match response {
      Ok(response) => Ok(Some(response.into_inner())),
      Err(status) if status.code() == Code::NotFound => Ok(None),
      Err(status) => Err(status_to_str(status)),
    }
  }
}