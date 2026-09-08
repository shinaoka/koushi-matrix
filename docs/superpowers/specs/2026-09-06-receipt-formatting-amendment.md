# Proposed receipt formatting boundary amendment

Status: approved and adopted into the surface/vertical. Sol (GPT-5.6, high,
read-only) first accepted ownership/boundedness with two material DTO/locale
findings; after the concrete clarification below, its recheck returned
**Correct-to-implement**. Timestamp formatting is the only amended responsibility.

## Why amend

Issue #840 assigns resolved display policy and scoped data selection to Rust;
it does not mandate that Rust run a locale formatting library. Its receipt
requirement is to remove ALL-reader eager formatting, not to replace native date
formatting. The i18n canon says Rust chooses locale/display profile and Core
returns structured codes/data instead of product UI prose.

Our approved receipt surface additionally required a formatted timestamp string
and the vertical said adapters do not format dates. That additional constraint
has led to a proposed platform service, timezone observers, and ICU/Jiff spikes
without advancing the required consumer path. Those have no implementation
approval and are not needed if native formatting remains a bounded adapter leaf.

## Proposed replacement contract

- Rust continues to own reader membership, order, exact totals, bounded window,
  identity/alias/avatar resolution, resource demand and revisions.
- ReaderRow keeps Rust-resolved display/original labels and initials. Replace
  `timestamp_label: Option<String>` with a typed optional timestamp value.
- The owning view provides a Rust-resolved receipt timestamp display policy:
  the existing resolved catalog locale and fixed medium-date/short-time style.
  Consumers do not select locale or style. There are no arbitrary format strings.
- The adapter converts that scalar presentation input using its native formatter.
  Current web adapter reuses Intl.DateTimeFormat with the same locale and
  dateStyle/timeStyle as today. A native/mobile adapter uses its native equivalent;
  the protocol does not require JavaScript, a WebView, or a Tauri URL.
- Formatting is only for delivered bounded rows, never ALL readers or hidden
  full-reader details. Compact rows do not eagerly format full-reader text.
- Core product semantics, selection, operation outcomes and revisions are fully
  testable without any formatter. Native formatting parity is adapter coverage.
- Preserve current host-local timezone interpretation and existing invalid-input
  behavior unless a separately reviewed bug fix is necessary. Do not add a Core
  timezone cache, OS discovery dependency or a new observer solely for this slice.
- Locale updates invalidate the affected view policy through the same direct
  scoped dependency path required for all row presentation changes. Adapter locale
  selection from raw settings remains prohibited.
- Native formatted strings are transient presentation, not authoritative Core
  records, ACK identities, source authorization or network-demand inputs.

This does NOT weaken scoped delivery, avatar ownership, accessibility, exact
counts, revision consistency or the requirement to remove the legacy full map.
It changes only which layer executes the already Rust-selected date style.

## Concrete timestamp and locale contract (review clarification)

Use `timestamp: Option<ReceiptTimestamp>` on ReaderRow. ReceiptTimestamp contains
`unix_ms: ReceiptTimestampMillis` and `locale: ReceiptTimestampLocale`.
ReceiptTimestampMillis is an unsigned whole millisecond count since Unix epoch,
valid inclusively from 0 through 8_640_000_000_000_000 (ECMAScript Date's positive
limit, below 2^53). Serialize it with the existing canonical decimal-string u64
codec plus range validation; JS can convert a validated value to Number exactly.
Malformed/noncanonical/out-of-range wire values reject the delivery without ACK,
not silently coerce. SDK timestamps absent or outside that supported range yield
None: no usable timestamp, while the reader/count remains present. This replaces
the legacy invalid-Date exception with an explicitly defined safe boundary; it
never drops a valid existing timestamp or a reader.

ReceiptTimestampLocale is a closed enum serialized as `en` or `ja`. Rust resolves
it from the existing LocaleDisplayProfile: catalog Ja → ja; En or Pseudo → en.
Pseudo dates are data, not catalog prose to accent or reverse; outer dir and
pseudo chrome continue to use the existing profile. Unsupported-language/RTL
profiles already select catalog En and consequently format dates as en. This
makes pseudo formatting deterministic instead of passing `pseudo` to Intl and
accidentally selecting the machine's ambient locale. Native consumers receive
these final BCP47 tags directly, without consulting settings or inventing a
fallback. Medium date / short time is fixed by ReceiptTimestamp's contract, not
an extensible format-string/configuration field. Host local timezone is unchanged.

En/ja formatting capability is an adapter requirement. If unexpectedly unavailable,
the adapter reports an explicit formatting error; it must not choose another
locale, silently omit the timestamp, or alter the authoritative reader model.
These clarifications received the Correct-to-implement verdict recorded above.

## Acceptance and adoption

Before coding: review whether this satisfies #839/#840 and the i18n ownership
boundary without silently reducing scope. If approved, amend the surface/vertical
and protocol model together; retire the unapproved formatter-port proposal and
spikes as superseded evidence, not alternative production paths.

Preserve existing locale/time display tests and add the changed DTO/adapter
boundary checks. Test en/ja and pseudo/RTL profile behavior, missing timestamps,
and verify that work is bounded by delivered rows. Native/core replica tests
compare structured timestamp/policy values, not browser-specific punctuation.
No assertion on product selection or lifecycle may be removed to accept this.
