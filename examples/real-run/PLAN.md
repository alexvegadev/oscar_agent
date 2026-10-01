# Execution Plan

## Goal

Document this interface: ...

## Work Mode

local

## Strategy

Local-first in mixed mode; strict provider boundaries otherwise. Workers produce proposals.

## Provider Strategy

Local: analysis, implementation proposals, tests and documentation when capable.
Remote: material reasoning gains, capability/context gaps and bounded escalation.

## Tasks

### T01 — Analyze interface definition

Preferred provider: local
Difficulty: Medium
Risk: Low
Dependencies: None

Inspect the provided interface definition to identify methods, parameters, return types, and behaviors. MISSING EVIDENCE: The interface definition is not provided in the input (placeholder '...' used). Uncertainty: Analysis cannot be completed without the source code or specification. Acceptance criteria: Identification of all components and constraints.

Reason: strict local mode

Context: Distilled; requirements: User-supplied interface definition

Expected outputs: Interface analysis report

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T02 — Define documentation requirements

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: T01

Determine the target audience and the required documentation format (e.g., Markdown, JSDoc, OpenAPI, or Swagger). Acceptance criteria: Selection of format, scope, and style guide.

Reason: strict local mode

Context: Distilled; requirements: Interface analysis report

Expected outputs: Documentation requirements specification

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T03 — Draft documentation

Preferred provider: local
Difficulty: Medium
Risk: Low
Dependencies: T01, T02

Generate the documentation content based on the analysis and requirements. Acceptance criteria: Complete documentation for all interface components, including examples and error cases.

Reason: strict local mode

Context: Distilled; requirements: Interface analysis report; Documentation requirements specification

Expected outputs: Draft documentation

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T04 — Review documentation

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: T03

Review the drafted documentation for accuracy, clarity, and completeness. Acceptance criteria: Finalized documentation ready for use.

Reason: strict local mode

Context: Distilled; requirements: Draft documentation

Expected outputs: Final documentation

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

## Execution Waves

### Wave 1

- T01

### Wave 2

- T02

### Wave 3

- T03

### Wave 4

- T04

## Expected Remote Calls

None initially.

Mixed-mode retries may add one remote call per eligible task. Actual calls and reported token usage appear in run.json. Estimates do not guarantee task correctness.
