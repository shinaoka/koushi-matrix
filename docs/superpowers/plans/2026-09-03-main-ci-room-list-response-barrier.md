# Main CI room-list response barrier

## Failure

Post-merge `main` run `33732993279` failed `normal_runtime_waits_for_full_all_rooms_reconciliation_and_reuses_one_sync_engine` at the shared five-second event-driven state watchdog. The fixture used 300/500ms Wiremock response delays as an implicit window for observing pre-response and partial-range state. Under runner load those delays consumed the watchdog without providing a causal release point.

## Change

Run this integration test on a two-thread Tokio runtime and give the responder explicit first-response and complete-range `std::sync::Barrier`s. For request 0 and 1, publish the existing request-index signal, then block response construction until the test inspects the corresponding state and releases the exact barrier. Remove all synthetic response delays, including reconnect/encryption delays; request-index and state-event channels already provide the required ordering.

Production code and the five-second deadlock watchdog remain unchanged.

## Verification

The GitHub failure is RED evidence. Run the exact integration test repeatedly and the complete `runtime_room_list_sync` target, formatting/boundary checks, independent diff review, required PR CI, merge, and post-merge `main` CI.
