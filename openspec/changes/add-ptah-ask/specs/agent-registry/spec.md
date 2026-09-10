# Agent Registry Capability Delta

## ADDED Requirements

### Requirement: Global ask section
Registry config files MAY define a top-level `[ask]` section — the registry's first global (non-agent) data — carrying a single `provider` key (string). The section SHALL be validated at discovery: a missing `provider` key or a value that is not a known provider name (`stdin` or `none` in v1) SHALL fail registry discovery with an error naming the file, the section, and the accepted values. Agent merge semantics are unchanged by the section's presence. Across layers, `[ask]` SHALL be replaced wholesale: when both project and user configs define `[ask]`, the project section wins in its entirety, and when only one layer defines it, that section applies. Provider credentials SHALL NOT be stored in registry files; v1's section has no settings beyond `provider`.

#### Scenario: Project section replaces user section wholesale
- **WHEN** the user config sets `[ask] provider = "none"` and the project config sets `[ask] provider = "stdin"`
- **THEN** the effective ask provider is `stdin`

#### Scenario: User-only section applies
- **WHEN** only the user config defines `[ask] provider = "none"` and the project config has no `[ask]` section
- **THEN** the effective ask provider is `none`

#### Scenario: Unknown provider value fails discovery
- **WHEN** a config file sets `[ask] provider = "stdni"`
- **THEN** registry discovery fails with an error naming the file and the accepted provider values

#### Scenario: Absent section parses cleanly
- **WHEN** a registry file defines only `[agents.*]` entries
- **THEN** it parses exactly as before; no ask configuration is contributed
