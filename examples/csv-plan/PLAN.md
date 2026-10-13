# Execution Plan

## Goal

Design a CSV importer with duplicate detection and tests

## Work Mode

local

## Strategy

Local-first in mixed mode; strict provider boundaries otherwise. Workers produce proposals.

## Provider Strategy

Local: analysis, implementation proposals, tests and documentation when capable.
Remote: material reasoning gains, capability/context gaps and bounded escalation.

## Tasks

### T01 — Analyze requirements and identify missing evidence

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: None

Analyze the goal to identify specific requirements for the CSV importer. Identify missing evidence: 1. Expected CSV schema (headers, data types). 2. Duplicate detection criteria (e.g., exact row match, unique ID column, or fuzzy match). 3. Error handling requirements (e.g., skip row, stop execution). Acceptance criteria: A requirements specification document listing identified requirements and missing evidence.

Reason: strict local mode

Context: Distilled; requirements: User-supplied evidence

Expected outputs: Requirements specification, Missing evidence list

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T02 — Design data structures and duplicate detection logic

Preferred provider: local
Difficulty: Medium
Risk: Low
Dependencies: T01

Design the internal data structures for representing CSV records and the algorithm for duplicate detection (e.g., using a hash set of unique keys or full row hashes). Acceptance criteria: A design proposal for data models and the duplicate detection algorithm.

Reason: strict local mode

Context: Distilled; requirements: Requirements specification

Expected outputs: Design proposal

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T03 — Implement CSV importer with duplicate detection

Preferred provider: local
Difficulty: Medium
Risk: Medium
Dependencies: T02

Implement the core CSV importer module. It must parse CSV files, handle headers, and identify duplicates based on the design in T02. Acceptance criteria: Functional code that parses CSVs and returns a list of unique records and a list of detected duplicates.

Reason: strict local mode

Context: Distilled; requirements: Design proposal

Expected outputs: Importer implementation

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T04 — Develop unit tests for importer

Preferred provider: local
Difficulty: Medium
Risk: Low
Dependencies: T03

Create a comprehensive suite of unit tests covering: 1. Valid CSV files. 2. Files with no duplicates. 3. Files with various duplicate types. 4. Malformed CSV rows. 5. Empty files. Acceptance criteria: All unit tests pass successfully.

Reason: strict local mode

Context: Distilled; requirements: Importer implementation

Expected outputs: Test suite, Test results

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T05 — Document the importer and its usage

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: T03

Create documentation including: 1. Installation/Setup instructions. 2. API reference for the importer class/functions. 3. Usage examples for different duplicate detection modes. Acceptance criteria: A complete README or documentation file.

Reason: strict local mode

Context: Distilled; requirements: Importer implementation

Expected outputs: Documentation file

Validation: NonEmpty

Escalation: at most 2 local attempts; remote permitted by task: false (work mode and budgets still apply).

### T06 — Review implementation and tests

Preferred provider: local
Difficulty: Low
Risk: Low
Dependencies: T03, T04, T05

Perform a final review of the importer code, the duplicate detection logic, and the test coverage to ensure they meet the requirements defined in T01. Acceptance criteria: A review report confirming the implementation is complete and correct.

Reason: strict local mode

Context: Distilled; requirements: Importer implementation; Test results; Documentation file

Expected outputs: Review report

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
- T05

### Wave 5

- T06

## Expected Remote Calls

None initially.

Mixed-mode retries may add one remote call per eligible task. Actual calls and reported token usage appear in run.json. Estimates do not guarantee task correctness.
