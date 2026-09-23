# Run Record Specification — Delta

## MODIFIED Requirements

### Requirement: Run ids are sortable and collision-free

The run id SHALL be the run's start instant in UTC formatted `yyyymmdd-hhmmss`
(e.g. `20260912-143022`), followed by `-` and a suffix of two lowercase words —
an adjective then a noun — joined by `-` (e.g. `20260912-143022-polite-aardvark`).
The timestamp prefix SHALL sort lexicographically in the order the runs
started, in every timezone; runs that start within the same second share that
prefix, and their relative order is unspecified. When a directory for a freshly
minted id already exists, ptah SHALL mint another id rather than reuse, merge
into, or overwrite that directory.

#### Scenario: Id shape
- **WHEN** a run starts at 2026-09-12T14:30:22Z
- **THEN** its id begins `20260912-143022-` and the remainder is two lowercase word tokens joined by `-`

#### Scenario: Ordering reflects start order across seconds
- **WHEN** two runs start in different seconds
- **THEN** ascending lexicographic order of their ids is the order they started

#### Scenario: Same-second runs share a prefix
- **WHEN** two runs start within the same second
- **THEN** their ids share the timestamp prefix and their relative sort order is unspecified

#### Scenario: UTC regardless of the machine's zone
- **WHEN** a run starts at local time 2026-09-12 21:30:22 in a UTC+7 zone
- **THEN** its id encodes `20260912-143022`, the UTC instant

#### Scenario: An occupied id is not reused
- **WHEN** the directory for a freshly minted id already exists
- **THEN** the record is written under a different id and the pre-existing directory is left untouched
