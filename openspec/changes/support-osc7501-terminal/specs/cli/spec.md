## ADDED Requirements

### Requirement: Viewing logs never replays terminal controls
When `devcroft logs` writes to a TTY, it SHALL replace every C0 and C1 control
character except newline and tab, including ESC (`0x1b`) and the 8-bit forms
(`0x80`–`0x9f`, and `U+0080`–`U+009F` in UTF-8), with a visible escape. Hook or
provisioning output recorded in the log therefore can't act as live OSC 7501
traffic or any other terminal control. Written to a pipe or a file, `logs`
SHALL leave the bytes unchanged.

#### Scenario: Recorded OSC sequence viewed on a terminal
- **WHEN** a sandbox hook wrote an OSC 7501 sequence to the log, and the user runs `devcroft logs` in a terminal
- **THEN** the sequence appears escaped, and the terminal receives no control sequence

#### Scenario: 8-bit control character
- **WHEN** the log holds a C1 OSC introducer (`0x9d`) and `devcroft logs` writes to a TTY
- **THEN** it is shown escaped, not passed through

#### Scenario: Piped output is unchanged
- **WHEN** `devcroft logs | cat` runs
- **THEN** the bytes match the log file
