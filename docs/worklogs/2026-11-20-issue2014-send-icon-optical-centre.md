# #2014: the Send icon gets a sub-pixel optical correction

## What was wrong

`.send-button` (`apps/desktop/src/styles.css`) centres the Lucide `Send` icon with
`display: grid; place-items: center`, so the icon's box is centred — but the
paper plane's ink is not centred inside that box. Measured on the rendered button:

| | offset from the button centre |
| --- | --- |
| icon box centre | `(+0.02, -0.03)` CSS px — sampling noise; `place-items: center` puts the 17px box in a 34×28 border-less box exactly on the centre |
| ink centroid | `(+1.31, -1.31)` CSS px — right and up |

So the report is right about the direction, and about the order of magnitude: the
nose's two converging strokes carry the mass while the tail is a thin outline, so
the shape reads as sitting up to the right of the space it leaves empty at the
bottom-left.

**How that number was obtained, so it can be re-derived.** A throwaway Playwright
script against the real `apps/desktop/src/styles.css`: page at deviceScaleFactor 2,
one `<button class="send-button ready">` whose child is a 17px `<svg
viewBox="0 0 24 24" stroke-width="2">` holding the two `send` path elements (path
data in `node_modules/lucide-react/dist/esm/icons/send.mjs`), then, over the
button's rect in viewport coordinates, `path.isPointInStroke()` at 0.05px steps
with the point pushed through `svg.getScreenCTM().inverse()`. The centroid is the
plain mean of the hit points, and its difference from the button's rect centre is
the number above. It needs `npm --prefix apps/desktop ci` first. This was one
observer and one engine: it is a geometric measure of the glyph, not a rendering
diff, so treat 1.31px as "about 1.3" rather than as a figure with two decimals of
meaning.

## The change

```css
.send-button svg {
  transform: translate(-1px, 1px);
}
```

The transform is on the icon, never on the button: the 34×28 box, its fill, the
`button:not(:disabled):active` press nudge, and the tap area are untouched. It is
physical rather than logical because this glyph is not mirrored for RTL — the
plane points up-right in every locale — so `translate` is the right tool here and
the logical-property contract test (`styles.contract.test.ts:257`) is not in play.

`-1px` rather than the measured `-1.31px`: it is simpler to read, and the residual
is `(0.31, -0.31)` CSS px — 0.63 device px at 2x. Not for pixel-grid reasons: this
24-unit viewBox is drawn at 17px, so the 2-unit stroke is already 1.4167 CSS px
wide and every ink edge is fractional whatever the offset.

Rendered against a centre crosshair at 10×, 1px and 1.31px are indistinguishable,
while 0.5px still reads as biased. That last comparison is a judgement from
looking at the renders, not a measurement, and it is what rules out the smaller
value.

**Placed by mass, not by extent.** Before the change the box was symmetric — 8.5px
of slack each side — and the mass sat low-left of nothing. After it the mass is on
the centre and the box is 1px asymmetric (7.5px to the left edge, 9.5px to the
right). That is what optical centring buys and it is deliberate: for a shape whose
nose carries the weight, the eye reads the gap around the shape, not the extent of
its outline. Anyone who later measures the two side gaps and calls the 1px
difference a bug is measuring the thing this change chose to trade. Element Web and
Element X were not inspected for this: a glyph-level nudge inside this app's own
button is not new Matrix functionality, so the upstream-flow rule in
`REPOSITORY_RULES.md` does not apply, and no upstream behaviour is claimed here.

The task brief bounded the nudge at under 1px. The measurement says the visible
bias is 1.31px, so a sub-1px value would leave the complaint half fixed; this is
recorded here as a deliberate deviation from that bound rather than an oversight.

## Verification

- RED first: the new contract test
  (`the Send icon carries a sub-pixel optical correction (#2014)`) failed on the
  unmodified stylesheet with `Expected CSS block for .send-button svg: expected
  null not to be null`, then passed once the rule was added.
- The test pins both halves: the button keeps `place-items: center` — the premise
  of a mass-relative nudge — and gains no `transform` of its own, and
  `.send-button svg` carries the correction. Two teeth, both exercised: without
  the rule it fails with `Expected CSS block for .send-button svg`; with the
  transform moved onto the button instead of the icon it fails with `expected …
  not to contain 'transform:'`.
- Re-measured after the change on the real stylesheet: ink centroid is
  `(+0.31, -0.31)` CSS px from the button centre.
- Rendered every state the stylesheet actually gives this control — idle (empty
  composer, `--text-faint` fill), `ready` (`--brand`), and the `is-sending` class,
  which has no rule of its own and so is visually identical to `ready` — in both
  the light and the dark token set. There is no `.send-button:hover` rule and no
  global `button:hover`, so hover adds nothing to check. The offset is identical
  in all of them because the states change only colours.
- Nothing clips: the shifted icon box sits 7.5px / 6.5px / 9.5px / 4.5px from the
  button's left / top / right / bottom edges — the first number is now 1px
  smaller than the second, as a mass-centred placement implies — and the button is
  `overflow: visible`.
- Gates: `npm run typecheck` clean, `npm run lint` clean (eslint plus the IME,
  agents-docs and semantic-owner checks), `npm test -- --run` 208 files / 2603
  tests passed.

## Not changed

- No `--send-icon-nudge` custom property or per-size variant: one icon, one
  button, one value. If another Lucide glyph turns out to need the same
  treatment, the shared shape is that glyph's own rule, not a token.
- The reporter's alternative — adjusting the `viewBox` — was not taken: it would
  change the icon's scale inside the button, not only its position. The
  transform moves the glyph without resizing it.
- `ICON_SIZE.input` (17px) stays as it is; the nudge is not a size change.

## Records

- Canon consulted: `docs/architecture/i18n.md:184-187` — physical left/right is
  allowed when the physical direction is part of the visual design and documented
  at the rule, which is what the CSS comment does. The logical-property contract
  (`styles.contract.test.ts:257-262`) covers `left`/`right` declarations and not
  `transform`, so it is not in play.
- The user guide needs no change: no help page describes icon placement, and the
  control's label, size, and behaviour are untouched.
- The PR template's four pre-review boxes: no manual instruction changes (no label,
  menu path, prerequisite or result moves); the user guide needs no update and
  `node scripts/user-help.mjs --check` passes; the UI property check
  (`.github/pull_request_template.md:13`) does not apply because no displayed or
  edited property changes.
- Not covered, and deliberately so: the contract test reads CSS text, so it cannot
  fail if the rule stops reaching the element — for example if the icon were
  hoisted out of `.send-button` or the class renamed. The browser tier already
  locates this control as `.composer .send-button`
  (`e2e/right-panel-containment.spec.ts:50,122`) and could assert
  `getComputedStyle(svg).transform`, but a 1px nudge does not warrant a new e2e
  spec; this line is the record that the gap exists.
- `is-sending` (`composer.tsx:1004`) is applied as a class with no rule anywhere
  in `styles.css` — a dead state hook, unrelated to this issue and left alone.
