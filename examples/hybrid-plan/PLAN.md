# Execution Plan

## Goal

Add JWT authentication to the API and protect admin endpoints.

## Work Mode

mixed

## Strategy

Local-first in mixed mode; strict provider boundaries otherwise. Workers produce proposals.

## Provider Strategy

Local: analysis, implementation proposals, tests and documentation when capable.
Remote: material reasoning gains, capability/context gaps and bounded escalation.

## Tasks

### T01 — Analyze available evidence

Preferred provider: local
Difficulty: Medium
Risk: High
Dependencies: None

Analyze the request and supplied evidence. Identify relevant interfaces and missing repository context; never claim files were inspected without evidence.

Reason: local-first; capability and context fit

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T01.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

### T02 — Choose design and constraints

Preferred provider: remote
Difficulty: High
Risk: High
Dependencies: T01

Use the analysis to resolve design trade-offs and risks. Record assumptions, constraints and validation criteria.

Reason: local capability or context gap

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T02.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

### T03 — Propose implementation

Preferred provider: local
Difficulty: Medium
Risk: High
Dependencies: T02

Produce an implementation proposal or patch artifact using the design and evidence. Do not claim changes were applied.

Reason: local-first; capability and context fit

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T03.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

### T04 — Propose validation cases

Preferred provider: local
Difficulty: Low
Risk: High
Dependencies: T02

Produce tests and expected outcomes for the design. Recommend deterministic checks; do not claim commands were executed.

Reason: local-first; capability and context fit

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T04.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

### T05 — Review sensitive proposals

Preferred provider: remote
Difficulty: High
Risk: High
Dependencies: T03, T04

Review implementation and test artifacts against the design. Identify contradictions and unresolved security risks.

Reason: reasoning quality gain justifies remote use

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T05.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

## Execution Waves

### Wave 1

- T01

### Wave 2

- T02

### Wave 3

- T03
- T04

### Wave 4

- T05

## Expected Remote Calls

- T02 — Choose design and constraints
- T05 — Review sensitive proposals

Mixed-mode retries may add one remote call per eligible task. Actual calls and reported token usage appear in run.json. Estimates do not guarantee task correctness.
