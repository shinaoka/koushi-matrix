# Superseded receipt formatter investigation

Status: superseded by the approved
[receipt formatting amendment](2026-09-06-receipt-formatting-amendment.md).
Do not implement the previously proposed native date-formatting port, Core
ICU/Jiff dependencies, or timezone observer for this slice. Rust supplies bounded
timestamp data and final locale/style policy; adapters execute native formatting.

The investigation did not change product dependencies or runtime configuration.
Its useful evidence remains:

- `/tmp/issue839-date-spike`: ICU4X 2.2/Jiff 0.2.35 source, lock and comparison.
  Native/Wasm compilation succeeded; ICU required provider `sync` for Send+Sync.
  36 en/ja/zone/DST cases agreed in date values, with 18 English U+202F versus
  ASCII-space differences. This was not exact output parity or application proof.
- `/tmp/issue839-timezone-spike`: native timezone investigation source/lock.
  Default IANA discovery ignored process TZ; the follow-up respected TZ and used
  existing Jiff parsers. 45 selected Linux conversions agreed with Intl. This was
  not a production adapter, cross-platform qualification or live-change proof.
- Build outputs for both spikes were removed; logs and small sources remain.

The initial port draft was not approved. The replacement amendment received
Sol's Correct-to-implement after explicit timestamp range/wire/error semantics and
formatter-ready locale policy were specified. Its adoption preserves existing
native timezone behavior without inventing another Core platform service.
