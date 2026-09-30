# Execution Plan

## Goal

Document a parser interface

## Work Mode

mixed

## Strategy

Local-first in mixed mode; strict provider boundaries otherwise. Workers produce proposals.

## Provider Strategy

Local: analysis, implementation proposals, tests and documentation when capable.
Remote: material reasoning gains, capability/context gaps and bounded escalation.

## Tasks

### T01 — Produce documentation

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: None

Produce a concise documentation artifact. State any missing source evidence.

Reason: local-first; capability and context fit

Context: Distilled; requirements: Goal and declared dependency artifacts; report missing repository evidence explicitly

Expected outputs: T01.md

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: true (work mode and budgets still apply).

## Execution Waves

### Wave 1

- T01

## Expected Remote Calls

None initially.

Mixed-mode retries may add one remote call per eligible task. Actual calls and reported token usage appear in run.json. Estimates do not guarantee task correctness.
