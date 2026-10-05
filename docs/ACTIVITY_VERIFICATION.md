# Activity verification

The current algorithm evaluates a user-uploaded recording against a running goal and reports recording consistency. It does **not** authenticate the runner or establish that an editable GPX/FIT file came directly from Garmin. There is no live Garmin API or AI verdict service in this flow.

## Import and calculations

The Rust parser in `crates/evidence/src/upload.rs` accepts GPX 1.1 and FIT files up to **16 MiB**, with bounded sample counts. It checks format, coordinates, ordered timestamps and bounded metrics/duration. Malformed files fail import. FIT parsing checks its format/CRC; a CRC detects corruption and is not a cryptographic signature. A FIT file with multiple sessions requires an explicit session selection.

GPX distance comes from timed coordinate intervals. FIT can also provide recorded cumulative distance, speed and session totals for cross-checking. Segment breaks are respected. Duration and average pace use elapsed recording time; FIT timer time is retained separately where available.

Heart rate and cadence summaries include sample count, missing/zero values, ranges and coverage. A missing measurement differs from zero. Cadence remains in the source unit; the upload display does not automatically double it into steps per minute. Missing sensors limit the checks that can be performed.

## Checks that request review

These broad prototype thresholds identify records worth examining; they do not diagnose health conditions or prove cheating.

| Check | Current trigger |
| --- | --- |
| Unusual speed | A measured short interval above 12 m/s |
| GPS jump | A coordinate interval exceeding both 100 m and 20 m/s |
| Sustained fast movement | At least 120 continuous seconds of 30-second distance windows above 8 m/s |
| Sparse sampling | Less than half the measured distance covered by intervals of at most 30 seconds |
| Sensor range | Positive heart rate outside 25–250 bpm, or raw cadence above 300 |
| Running signal mismatch | At least 120 continuous seconds above 4 m/s with recorded zero cadence and positive heart rate below 100 bpm at both interval endpoints; intervals must be at most 30 seconds, in one segment, with both sensors and distance |
| FIT session consistency | Sample/session timestamp disagreement above 5 seconds; elapsed-time discrepancy above max(2 seconds, 2%); distance discrepancy above max(100 m, 5%) |
| Independent FIT signals | Complete GPS versus recorded distance discrepancy above max(100 m, 10%), or comparable integrated recorded speed versus distance discrepancy above max(200 m, 25%) |

Gaps, missing sensors and nonmatching intervals reset a sustained signal-mismatch period. A long stationary pause is not itself invented movement. Checks with insufficient independent data are shown as limited, rather than automatically classified as fraud. The policy identifier is `manual-running-plausibility-v3`.

## Goal eligibility and outcomes

The API also compares the measured distance with the challenge target. In **LIVE / Record a new run** mode the whole recording must fall within the agreed activity window. In **REPLAY / Use a saved run** mode historical recordings are allowed for prototype presentation. Both modes still enforce upload timing and reject future recordings.

| Outcome | Meaning and next step |
| --- | --- |
| Accepted | Distance and applicable activity window are met, with no review-triggering consistency finding. Success settlement or the company reward claim becomes available. |
| Not accepted | The goal distance or time window is not met. The recording can still be valid; it is not counted for this particular goal. Upload another eligible recording before the deadline. |
| Review required | The goal is met, but a consistency finding needs an authorized human decision before it counts. Review records the decision and explanation. |
| Import error | The file cannot be parsed safely, a FIT session must be selected, or another upload prerequisite is unmet. Resolve the reported cause before retrying. |

An operator cannot accept a recording that did not meet the target/window solely by changing its review outcome. An accepted upload establishes eligibility under the prototype's rules, not independent proof of identity.

## Reuse, privacy and settlement

File hashes and a normalized activity fingerprint help identify repeated recordings. Repeating the same upload in one personal challenge returns its existing record rather than creating another reward. Eligible recordings cannot be reused by the same user across personal challenges and company participation. These checks do not guarantee detection of every deliberately modified or fabricated file.

The source recording and telemetry remain private off-chain. Participants can inspect their recorded checks; employers see progress/results instead of raw GPS or biometric samples. Uploading a file does not move tokens by itself: the separate LOCAL or Devnet settlement action applies the eligible outcome. Solana enforces token custody and authorized transactions; it does not run these telemetry checks.
