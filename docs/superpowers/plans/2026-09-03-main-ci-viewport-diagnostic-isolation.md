# Main CI viewport diagnostic isolation

## Failure

PR #804's current-main CI run `33720343157` failed in `viewport_sync::tests::generation_is_monotonic_and_diagnostic_receipt_is_closed`: the test indexed the first global diagnostic appended after its snapshot and assumed it belonged to the test. The bounded diagnostic ring can remain at constant length when it evicts an old record, leaving that suffix empty; parallel tests can also append unrelated records because not every production diagnostic uses the test lock.

## Change

Keep production diagnostics unchanged. In the viewport test, search the bounded snapshot newest-first for the exact `desktop.viewport_sync` / `observed` event with generation 2, then assert its generation and redaction fields. Apply the same root-cause fix to the adjacent media-summary test: find the newest exact `summary` event for each source instead of slicing from the old ring length. Do not add retries, sleeps, serialization, or timeouts.

## Verification

The CI failure is the RED evidence. Run the exact focused test repeatedly, then `cargo test -p koushi-desktop --lib`, formatting, diagnostic-isolation guard, and required PR CI. Merge independently before refreshing PR #804.
