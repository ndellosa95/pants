# Copyright 2023 Pants project contributors (see CONTRIBUTORS.md).
# Licensed under the Apache License, Version 2.0 (see LICENSE).

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from pants.backend.python.lint.ruff.check.skip_field import SkipRuffCheckField
from pants.backend.python.lint.ruff.common import RunRuffRequest, run_ruff
from pants.backend.python.lint.ruff.skip_field import SkipRuffField
from pants.backend.python.lint.ruff.subsystem import Ruff, RuffMode
from pants.backend.python.target_types import (
    InterpreterConstraintsField,
    PythonResolveField,
    PythonSourceField,
)
from pants.backend.python.util_rules import pex
from pants.core.goals.fix import FixResult, FixTargetsRequest
from pants.core.goals.lint import REPORT_DIR, LintResult, LintTargetsRequest
from pants.core.util_rules.partitions import PartitionerType
from pants.core.util_rules.source_files import SourceFilesRequest, determine_source_files
from pants.engine.fs import DigestSubset, PathGlobs, RemovePrefix
from pants.engine.intrinsics import digest_subset_to_digest, remove_prefix
from pants.engine.platform import Platform
from pants.engine.rules import collect_rules, rule
from pants.engine.target import FieldSet, Target
from pants.util.logging import LogLevel
from pants.util.meta import classproperty


@dataclass(frozen=True)
class RuffCheckFieldSet(FieldSet):
    required_fields = (PythonSourceField,)

    source: PythonSourceField
    resolve: PythonResolveField
    interpreter_constraints: InterpreterConstraintsField

    @classmethod
    def opt_out(cls, tgt: Target) -> bool:
        return tgt.get(SkipRuffCheckField).value or tgt.get(SkipRuffField).value


class RuffLintRequest(LintTargetsRequest):
    field_set_type = RuffCheckFieldSet
    tool_subsystem = Ruff
    partitioner_type = PartitionerType.DEFAULT_SINGLE_PARTITION

    @classproperty
    def tool_name(cls) -> str:
        return "ruff check"

    @classproperty
    def tool_id(cls) -> str:
        return "ruff-check"


class RuffFixRequest(FixTargetsRequest):
    field_set_type = RuffCheckFieldSet
    tool_subsystem = Ruff
    partitioner_type = PartitionerType.DEFAULT_SINGLE_PARTITION

    # We don't need to include automatically added lint rules for this RuffFixRequest,
    # because these lint rules are already checked by RuffLintRequest.
    enable_lint_rules = False

    @classproperty
    def tool_name(cls) -> str:
        return "ruff check --fix"

    @classproperty
    def tool_id(cls) -> str:
        return RuffLintRequest.tool_id


@rule(desc="Fix with `ruff check --fix`", level=LogLevel.DEBUG)
async def ruff_fix(request: RuffFixRequest.Batch, ruff: Ruff, platform: Platform) -> FixResult:
    result = await run_ruff(
        RunRuffRequest(snapshot=request.snapshot, mode=RuffMode.FIX),
        ruff,
        platform,
    )
    return await FixResult.create(request, result)


@rule(desc="Lint with `ruff check`", level=LogLevel.DEBUG)
async def ruff_lint(
    request: RuffLintRequest.Batch[RuffCheckFieldSet, Any],
    ruff: Ruff,
    platform: Platform,
) -> LintResult:
    source_files = await determine_source_files(
        SourceFilesRequest(field_set.source for field_set in request.elements)
    )
    result = await run_ruff(
        RunRuffRequest(snapshot=source_files.snapshot, mode=RuffMode.LINT),
        ruff,
        platform,
    )
    report_digest = await digest_subset_to_digest(
        DigestSubset(result.output_digest, PathGlobs([f"{REPORT_DIR}/**"]))
    )
    report = await remove_prefix(RemovePrefix(report_digest, REPORT_DIR))
    return LintResult.create(request, result, report=report)


def rules():
    return (
        *collect_rules(),
        *RuffFixRequest.rules(),
        *RuffLintRequest.rules(),
        *pex.rules(),
    )
