# ironpress — CSS/HTML feature & parity coverage tracker
> Living master doc. Tracks every PDF-relevant CSS/HTML feature, ironpress support + test status, and every coverage gap across adversarial spec-audit rounds. Generated from the spec-driven finder rounds; the **Verdict** column is filled in by the patch+verify render of each gap.
## Summary
- **Features audited:** 499 (across 8 areas)
- **Feature status:** supported-tested 223, partial 112, unsupported 102, supported-untested 45, na-not-pdf-relevant 17
- **Coverage gaps (deduped):** 96 — kinds: support-gap 73, test-gap 17, interaction-flow 6
- **Gap expectation:** fail 79, pass 17 (expected_ironpress: fail = candidate real bug, pass = test-gap)
- **Find rounds so far:** 1

## Coverage gap log (all rounds)
Verdict legend: `PENDING` (not yet rendered) · `LOCKED` (ironpress correct, now a regression test) · `REAL-BUG` (ironpress wrong vs spec-correct oracle; tracked-unsupported until fixed) · `DROPPED` (oracle can't test / not discriminating).

| # | area | category | feature | kind | expect | oracle | prio | verdict | fix-effort | notes |
|---|------|----------|---------|------|--------|--------|------|---------|-----------|-------|
| 1 | grid | grid | repeat(auto-fill, fixed track) | support-gap | fail | chrome | 5 | PENDING |  | A parser that expands auto-fill to a constant three columns still passes the current fixed |
| 2 | grid | grid | repeat(auto-fit, minmax()) empty-track collapse | support-gap | fail | chrome | 5 | PENDING |  | Treating auto-fit as a fixed repeat count, or as auto-fill without empty-track collapse, i |
| 3 | grid | grid | grid-auto-columns on implicit columns | support-gap | fail | chrome | 5 | PENDING |  | An engine that only creates implicit rows still passes the current grid-auto-rows fixture. |
| 4 | grid | grid | order on grid items | interaction-flow | fail | chrome | 5 | PENDING |  | Ignoring order still passes source-order grid fixtures and the existing flex-only order co |
| 5 | grid | grid | z-index on static overlapping grid items | interaction-flow | fail | chrome | 5 | PENDING |  | A table-like grid emitter that skips overlaps still passes all current non-overlapping gri |
| 6 | grid | grid | grid-template-columns: subgrid | support-gap | fail | chrome | 5 | PENDING |  | Treating the nested grid as an independent one-column grid is invisible until a subgrid fi |
| 7 | borders-bg-gradients | backgrounds-borders | background-origin:border-box | test-gap | pass | chrome | 4 | PENDING |  | Defaulting every image to padding-box would pass the existing content-box origin fixture b |
| 8 | filters-effects-clip-color | effects | background-blend-mode: overlay | test-gap | pass | chrome | 4 | PENDING |  | A PDF blend-mode mapping that accidentally supports only multiply can pass the current bac |
| 9 | grid | grid | grid-template-columns: fit-content(<length>) | support-gap | fail | chrome | 4 | PENDING |  | Approximating fit-content() as auto lets the first track grow to max-content and still pas |
| 10 | grid | grid | min-content and max-content track sizing keywords | support-gap | fail | chrome | 4 | PENDING |  | Mapping both intrinsic keywords to auto makes them the same width and is not challenged by |
| 11 | grid | grid | grid-auto-flow: row dense | test-gap | pass | chrome | 4 | PENDING |  | The current column-flow fixture cannot distinguish sparse row placement from dense backfil |
| 12 | grid | grid | grid-column-start/end and grid-row-start/end longhands | support-gap | fail | chrome | 4 | PENDING |  | Supporting only grid-column/grid-row shorthands passes the existing line-number fixtures b |
| 13 | grid | grid | grid-template shorthand with area strings | support-gap | fail | chrome | 4 | PENDING |  | Engines that only implement grid-template-areas/rows/columns longhands pass the current ma |
| 14 | text-inline-fonts-generated | lists-counters | counter-set | support-gap | fail | chrome | 4 | PENDING |  | Supporting only counter-reset/increment passes current counter fixtures but cannot set an  |
| 15 | tables | tables | border-spacing interaction with border-collapse:collapse | test-gap | pass | chrome | 4 | PENDING |  | Current fixtures cover collapse and spacing separately, so an implementation that forgets  |
| 16 | tables | tables | colgroup/col span width assignment | test-gap | pass | chrome | 4 | PENDING |  | A renderer that handles only simple colgroup widths but ignores span could pass the existi |
| 17 | borders-bg-gradients | backgrounds-borders | background-size: auto <length> | support-gap | fail | chrome | 3 | PENDING |  | Treating the value as plain auto preserves the intrinsic 40 by 20 size and still passes co |
| 18 | filters-effects-clip-color | filters | filter: url(#id) with feGaussianBlur | support-gap | fail | chrome | 3 | PENDING |  | Only recognizing feColorMatrix or silently ignoring unknown SVG filter primitives passes t |
| 19 | flexbox | flexbox | abspos child of flex container | support-gap | fail | chrome | 3 | PENDING |  | Treating the abspos child as simply anchored at the padding-box origin passes normal flex  |
| 20 | flexbox | flexbox | align-content with flex-direction:column and flex-wrap:wrap | test-gap | pass | chrome | 3 | PENDING |  | Row-only align-content code passes all current align-content fixtures and the column-wrap  |
| 21 | flexbox | flexbox | cross-axis auto margins | test-gap | pass | chrome | 3 | PENDING |  | The current margin:auto fixture checks symmetric centering; an implementation could specia |
| 22 | text-inline-fonts-generated | fonts-advanced | font-variant-caps:small-caps | test-gap | pass | chrome | 3 | PENDING |  | Ignoring font-variant passes current font family/size/weight fixtures; the small-caps mani |
| 23 | grid | grid | grid-template-areas validation | support-gap | fail | chrome | 3 | PENDING |  | A lenient parser that computes the bounding box of an L-shaped area passes all current val |
| 24 | text-inline-fonts-generated | lists-counters | list-style-image:url(data:png) | test-gap | pass | chrome | 3 | PENDING |  | Falling back to list-style-type disc passes current marker type fixtures and the manifest' |
| 25 | transforms-pos-overflow-images-units-box | overflow-clipping | overflow-x:visible with overflow-y:hidden | test-gap | pass | chrome | 3 | PENDING |  | A renderer that preserves visible on the x axis lets the child spill right, while current  |
| 26 | tables | tables | table-layout:auto width distribution with colspan | support-gap | fail | chrome | 3 | PENDING |  | Evenly dividing the spanning cell's width across both columns over-expands the already-wid |
| 27 | tables | tables | rowspan=0 table-cell spanning | support-gap | fail | chrome | 3 | PENDING |  | Clamping rowspan=0 to 1 makes the later rows shift into the first column, so ordinary posi |
| 28 | tables | tables | table-cell vertical-align value applicability | support-gap | fail | chrome | 3 | PENDING |  | Treating text-bottom like bottom moves the second cell's content to the bottom of the row, |
| 29 | text-inline-fonts-generated | text-advanced | tab-size:<number> with white-space:pre | test-gap | pass | chrome | 3 | PENDING |  | Leaving tabs at the default width or collapsing them passes current white-space fixtures b |
| 30 | transforms-pos-overflow-images-units-box | units-values | ch font-relative length in layout | support-gap | fail | chrome | 3 | PENDING |  | Resolving ch as 0.5em passes em/rem fixtures but makes the metric-dependent bar too short. |
| 31 | transforms-pos-overflow-images-units-box | units-values | width:min-content, width:max-content, vmin, vmax, and aspect | test-gap | pass | chrome | 3 | PENDING |  | Treating min/max-content as auto, resolving vmin/vmax against a default viewport, or dropp |
| 32 | borders-bg-gradients | backgrounds-borders | border-style: groove ridge inset outset | support-gap | fail | chrome | 2 | PENDING |  | Mapping unsupported style keywords to solid preserves dimensions and passes existing dashe |
| 33 | borders-bg-gradients | backgrounds-borders | background-repeat: space round | support-gap | fail | chrome | 2 | PENDING |  | Treating unknown repeat keywords as repeat still paints tiles, so ordinary repeat/no-repea |
| 34 | borders-bg-gradients | backgrounds-borders | box-shadow with border-radius:50% on a non-square box | interaction-flow | fail | chrome | 2 | PENDING |  | Using a scalar radius for shadows creates a rounded rectangle with flat segments; simple b |
| 35 | borders-bg-gradients | backgrounds-gradients | linear-gradient color interpolation hints | support-gap | fail | chrome | 2 | PENDING |  | Ignoring hints or rejecting the gradient yields either a normal 50% midpoint or no gradien |
| 36 | transforms-pos-overflow-images-units-box | block-box-model | display:contents | support-gap | fail | chrome | 2 | PENDING |  | Treating display:contents as block paints the wrapper background and padding; current fixt |
| 37 | transforms-pos-overflow-images-units-box | block-box-model | display:flow-root block formatting context | support-gap | fail | chrome | 2 | PENDING |  | Ignoring flow-root leaves a normal block with zero height around its float, so the followi |
| 38 | filters-effects-clip-color | color-opacity | opacity group rendering for text glyphs | interaction-flow | fail | chrome | 2 | PENDING |  | Applying alpha per glyph draw instead of first rendering the element into an isolated grou |
| 39 | filters-effects-clip-color | color-opacity | oklch() | support-gap | fail | chrome | 2 | PENDING |  | A parser limited to sRGB legacy syntaxes passes the current color fixtures but drops or de |
| 40 | filters-effects-clip-color | effects | mix-blend-mode: luminosity | support-gap | fail | chrome | 2 | PENDING |  | Treating unknown blend modes as normal passes current multiply/screen-only fixtures. |
| 41 | filters-effects-clip-color | filters | filter: drop-shadow(<length>{2,3}) default color | support-gap | fail | chrome | 2 | PENDING |  | Defaulting the omitted drop-shadow color to black passes current explicit-color drop-shado |
| 42 | flexbox | flexbox | flex-basis:max-content | support-gap | fail | chrome | 2 | PENDING |  | A parser that accepts only length/percentage/auto/content ignores max-content and falls ba |
| 43 | flexbox | flexbox | align-items:baseline | support-gap | fail | chrome | 2 | PENDING |  | A baseline implementation that only handles text items passes current text-baseline fixtur |
| 44 | flexbox | flexbox | fragmenting flex layout | support-gap | fail | chrome | 2 | PENDING |  | All current fixtures fit on one page, so an engine can treat a FlexRow as atomic and still |
| 45 | flexbox | flexbox | automatic minimum size | test-gap | pass | chrome | 2 | PENDING |  | The current min-content fixture only checks the default floor; an implementation that alwa |
| 46 | text-inline-fonts-generated | generated-content | ::first-line { font-size: ... } | support-gap | fail | chrome | 2 | PENDING |  | Restyling only color/background/bold passes current ::first-line fixtures but leaves first |
| 47 | text-inline-fonts-generated | inline-text | vertical-align:text-bottom | test-gap | pass | chrome | 2 | PENDING |  | Aliasing text-bottom to bottom passes current vertical-align fixtures because text-bottom  |
| 48 | text-inline-fonts-generated | inline-text | text-indent:<percentage> | support-gap | fail | chrome | 2 | PENDING |  | A length-only text-indent implementation ignores the declaration and still passes current  |
| 49 | fragmentation-paged | multicol | column-gap: <percentage> | support-gap | fail | chrome | 2 | PENDING |  | An implementation that parses percentages but drops the stored percentage during layout wi |
| 50 | fragmentation-paged | multicol | column-rule painting condition | support-gap | fail | chrome | 2 | PENDING |  | A renderer that draws rules for every column gap whenever column-count > 1 will pass fixtu |
| 51 | transforms-pos-overflow-images-units-box | overflow-clipping | overflow:clip with overflow-clip-margin | support-gap | fail | chrome | 2 | PENDING |  | Clipping exactly at the padding box passes current hidden/clip tests but removes the spec- |
| 52 | fragmentation-paged | paged-media | break-after:right | test-gap | pass | chrome | 2 | PENDING |  | An implementation that treats right as ordinary page, or aliases right to always, would st |
| 53 | fragmentation-paged | paged-media | table header/footer group repetition during page fragmentati | test-gap | pass | chrome | 2 | PENDING |  | A table paginator that can split tall rows but forgets to repeat header/footer groups woul |
| 54 | fragmentation-paged | paged-media | flex fragmentation across pages | support-gap | fail | chrome | 2 | PENDING |  | A paginator that treats a flex row/container as unsplittable can pass block/table paginati |
| 55 | transforms-pos-overflow-images-units-box | selectors-cascade | @layer ordering and revert-layer family | support-gap | fail | chrome | 2 | PENDING |  | A source-order-only cascade or an unknown-at-rule dropper passes current specificity/sourc |
| 56 | tables | tables | missing cells fixup | support-gap | fail | chrome | 2 | PENDING |  | A renderer that simply stops at the last authored cell never creates the two anonymous cel |
| 57 | tables | tables | empty-cells: hide with visibility:hidden and all-empty row b | support-gap | fail | chrome | 2 | PENDING |  | A DOM-presence emptiness test treats hidden text as content, so it still paints the hidden |
| 58 | tables | tables | table-layout: fixed and cell overflow handling | support-gap | fail | chrome | 2 | PENDING |  | A width-only fixed-layout implementation sizes the columns correctly but lets cell text pa |
| 59 | text-inline-fonts-generated | text-advanced | text-align:end with direction:rtl | support-gap | fail | chrome | 2 | PENDING |  | Treating unknown text-align values as left or as direction start is invisible to current l |
| 60 | text-inline-fonts-generated | text-advanced | writing-mode:vertical-rl | test-gap | pass | chrome | 2 | PENDING |  | Ignoring writing-mode passes all horizontal text fixtures and the existing vertical-rl kno |
| 61 | borders-bg-gradients | backgrounds-borders | border-image-source/slice/repeat shorthand | support-gap | fail | chrome | 1 | PENDING |  | An engine that ignores border-image or falls back to the transparent physical border would |
| 62 | borders-bg-gradients | backgrounds-borders | background-position: right 20px bottom 15px | support-gap | fail | chrome | 1 | PENDING |  | A parser that only supports one/two-value positions silently uses the default top-left pos |
| 63 | borders-bg-gradients | backgrounds-borders | background-clip:text | support-gap | fail | chrome | 1 | PENDING |  | Mapping text to border-box paints a rectangular gradient and leaves transparent text invis |
| 64 | borders-bg-gradients | backgrounds-gradients | repeating-linear-gradient with px color stops | support-gap | fail | chrome | 1 | PENDING |  | A parser that only accepts percent stops passes existing percentage repeating-gradient fix |
| 65 | borders-bg-gradients | backgrounds-gradients | rgba()/transparent color stops in gradients | support-gap | fail | chrome | 1 | PENDING |  | Dropping alpha makes the whole gradient opaque red; current gradient fixtures use opaque c |
| 66 | borders-bg-gradients | backgrounds-gradients | multiple gradient backgrounds with per-layer background-size | support-gap | fail | chrome | 1 | PENDING |  | Keeping only one gradient slot or applying the first layer's geometry to the second layer  |
| 67 | filters-effects-clip-color | clip-mask | clip-path: inset(0) content-box | support-gap | fail | chrome | 1 | PENDING |  | Parsing only bare inset() and ignoring geometry-box keywords passes current simple clip-pa |
| 68 | filters-effects-clip-color | clip-mask | clip-path: polygon(evenodd, ...) | support-gap | fail | chrome | 1 | PENDING |  | Ignoring the fill-rule or always using nonzero clipping passes current solid polygon fixtu |
| 69 | filters-effects-clip-color | clip-mask | mask-image layers with mask-composite: exclude | support-gap | fail | chrome | 1 | PENDING |  | Keeping only the first mask-image layer or ignoring mask-composite passes current single-m |
| 70 | filters-effects-clip-color | color-opacity | opacity: <percentage> | support-gap | fail | chrome | 1 | PENDING |  | A parser that accepts only number opacity values passes current opacity fixtures, which us |
| 71 | filters-effects-clip-color | filters | filter: grayscale() on a container group | support-gap | fail | chrome | 1 | PENDING |  | An implementation that recolors only the parent's own background/border, or ignores text a |
| 72 | filters-effects-clip-color | filters | filter function chains: contrast() before blur() versus blur | interaction-flow | fail | chrome | 1 | PENDING |  | An implementation that always applies blur before color filters will make both boxes look  |
| 73 | flexbox | flexbox | display:inline-flex | support-gap | fail | chrome | 1 | PENDING |  | A parser that only accepts display:flex silently leaves inline-flex as ordinary inline con |
| 74 | flexbox | flexbox | anonymous flex items | support-gap | fail | chrome | 1 | PENDING |  | An implementation that only iterates element children drops or ignores direct text nodes;  |
| 75 | flexbox | flexbox | flex-direction row logical axis mapping | support-gap | fail | chrome | 1 | PENDING |  | A physical left-to-right row algorithm passes all current row and row-reverse fixtures bec |
| 76 | flexbox | flexbox | justify-content:safe center | support-gap | fail | chrome | 1 | PENDING |  | A parser that strips safe and always applies unsafe center passes the current safe-center  |
| 77 | flexbox | flexbox | aspect-ratio flex items | support-gap | fail | chrome | 1 | PENDING |  | An implementation that computes aspect-ratio height before flex-grow sees a zero basis and |
| 78 | transforms-pos-overflow-images-units-box | images-replaced | object-position:right 20px bottom 10px with object-fit:cover | support-gap | fail | chrome | 1 | PENDING |  | A parser that treats right 20px as plain right alignment passes percent, keyword, and near |
| 79 | text-inline-fonts-generated | inline-text | vertical-align:<length> | support-gap | fail | chrome | 1 | PENDING |  | A keyword-only vertical-align implementation treats the declaration as baseline and still  |
| 80 | fragmentation-paged | multicol | break-before: column | support-gap | fail | chrome | 1 | PENDING |  | A multicol implementation that only supports automatic filling and page breaks, but ignore |
| 81 | fragmentation-paged | paged-media | break-after:left plus break-before:right at the same break p | interaction-flow | fail | chrome | 1 | PENDING |  | A paginator that emits two independent page-break tokens and suppresses the second one on  |
| 82 | fragmentation-paged | paged-media | @page :blank | support-gap | fail | weasyprint | 1 | PENDING |  | A renderer that creates the blank page but never applies :blank rules would pass current p |
| 83 | fragmentation-paged | paged-media | @left-middle and @right-middle page margin boxes | support-gap | fail | weasyprint | 1 | PENDING |  | A renderer that only supports top and bottom margin boxes will pass existing counter/heade |
| 84 | fragmentation-paged | paged-media | named @page margin boxes | support-gap | fail | weasyprint | 1 | PENDING |  | A renderer that applies named-page size/margins but collects only unqualified @page margin |
| 85 | fragmentation-paged | paged-media | footnote-policy: line plus footnote-display:inline | support-gap | fail | weasyprint | 1 | PENDING |  | A footnote implementation that only extracts note bodies and always lays them as block not |
| 86 | fragmentation-paged | paged-media | string-set/string() and element(name,last) | support-gap | fail | weasyprint | 1 | PENDING |  | A renderer that supports only element(name) with a default selection and ignores string-se |
| 87 | transforms-pos-overflow-images-units-box | positioning | position:fixed in paged media | support-gap | fail | chrome | 1 | PENDING |  | Treating fixed as single absolute positioning passes one-page fixed fixtures but omits the |
| 88 | transforms-pos-overflow-images-units-box | selectors-cascade | @supports selector(), :has(> .flag), and :nth-child(An+B of  | support-gap | fail | chrome | 1 | PENDING |  | Sibling-only :has and nth-child without the of-selector clause pass the current selector f |
| 89 | tables | tables | border-collapse conflict resolution: hidden wins | support-gap | fail | chrome | 1 | PENDING |  | A renderer that parses hidden as none or paints every cell edge independently will draw th |
| 90 | tables | tables | border-collapse conflict resolution: border width precedence | support-gap | fail | chrome | 1 | PENDING |  | Painting cell borders in DOM order instead of choosing the conflict winner leaves a narrow |
| 91 | tables | tables | border-collapse conflict resolution: border style precedence | support-gap | fail | chrome | 1 | PENDING |  | A renderer that overpaints both borders in cell order will render the later solid red edge |
| 92 | tables | tables | display: table/table-row/table-cell plus anonymous table fix | support-gap | fail | chrome | 1 | PENDING |  | A tag-only table implementation ignores display:table and stacks the div children as norma |
| 93 | text-inline-fonts-generated | text-advanced | hyphens:manual and U+00AD soft hyphen | support-gap | fail | chrome | 1 | PENDING |  | A wrapper that only breaks at spaces or arbitrary overflow-wrap points passes current brea |
| 94 | transforms-pos-overflow-images-units-box | transforms | translate/rotate individual transform properties plus transf | support-gap | fail | chrome | 1 | PENDING |  | A renderer that only parses transform:... and always uses the border box for origins passe |
| 95 | transforms-pos-overflow-images-units-box | transforms | rotateY(180deg) with backface-visibility:hidden | support-gap | fail | chrome | 1 | PENDING |  | A 2D approximation of rotateY as scaleX(-1), or an implementation that ignores backface-vi |
| 96 | text-inline-fonts-generated | typography | line-height:<percentage> | support-gap | fail | chrome | 1 | PENDING |  | Ignoring percentage line-height falls back to inherited 1.5 and passes current number/leng |

## Feature matrix by area

### borders-bg-gradients
_The target manifests already cover core solid/rgba backgrounds, basic linear/radial/conic gradients, percentage hard stops, repeating gradient families, common background-position/size/origin cases, solid/dashed/dotted/double/none borders, per-side colors, border-width keywords, many border-radius expansions including percentages/ellipses/clamping, outline offsets, and simple box-shadow. The blind spots are concentrated in unimplemented CSS Backgrounds & Borders values (border-image, 3D border styles, attachment, repeat space/round, four-value position, mixed auto sizing), in arbitrary multi-layer backgrounds and per-layer geometry, and in gradient stop semantics/alpha plus radius propagation into clips and shadows._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| background shorthand: color, image, position/size, repeat, o | CSS Backgrounds & Borders 3 se | partial | src/parser/css/inline.rs parses background shorthand but tracks one ra |
| background-color including solid colors, rgba(), transparent | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: background-color-solid, background-color |
| background-image: none and url() raster/SVG images | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-gradients: background-position-keyword, backgroun |
| image-set() as a background image with resolution selection | CSS Images 4 image-set() | unsupported | MDN image-set consulted as shipped; rg found no image-set parsing in s |
| multiple background image layers with CSS list matching | CSS Backgrounds & Borders 3 se | partial | manifest backgrounds-gradients: multiple-backgrounds-layered expected_ |
| per-layer background-position/background-size/background-rep | CSS Backgrounds & Borders 3 se | partial | src/parser/css/inline.rs records background-layer-slots but computed s |
| background-position one- and two-value syntax with keywords, | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-gradients: background-position-keyword; src/style |
| background-position three-/four-value edge-offset syntax suc | CSS Backgrounds & Borders 3 se | unsupported | src/style/computed.rs parse_background_position handles len==1 and len |
| background-size cover, contain, explicit length/percentage p | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-gradients: background-size-cover, background-size |
| background-size mixed auto/length or auto/percentage pairs | CSS Backgrounds & Borders 3 se | partial | src/style/computed.rs parse_background_size accepts exact auto or expl |
| background-repeat repeat, no-repeat, repeat-x, repeat-y | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-gradients image sizing/position fixtures exercise |
| background-repeat round and space | CSS Backgrounds & Borders 3 se | unsupported | src/style/computed.rs BackgroundRepeat has no Round/Space variants; pa |
| background-origin border-box, padding-box, content-box | CSS Backgrounds & Borders 3 se | supported-untested | manifest backgrounds-gradients covers background-origin-content-box on |
| background-clip border-box, padding-box, content-box | CSS Backgrounds & Borders 3 se | partial | manifest backgrounds-gradients: background-clip-padding-box expected_s |
| background-clip:text with text-shaped background painting | CSS Backgrounds & Borders 4/MD | unsupported | src/style/computed.rs parse_background_clip maps text to Border instea |
| background-attachment scroll/fixed/local | CSS Backgrounds & Borders 3 se | unsupported | rg background-attachment in src/ found no parser/computed/render suppo |
| root/body canvas background propagation | CSS Backgrounds & Borders 3 se | supported-untested | src/lib.rs has root/body canvas background handling; no target manifes |
| border-width longhands/shorthand including thin/medium/thick | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-solid-width, border-width-keyword |
| border-color longhands/shorthand including per-side, transpa | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-per-side-colors, border-color-tra |
| border-style none, hidden, solid, dashed, dotted, double | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-style-dashed, border-style-dotted |
| border-style groove, ridge, inset, outset 3D rendering | CSS Backgrounds & Borders 3 se | partial | src/style/computed.rs parse_border_style_keyword does not preserve gro |
| per-side border shorthand/longhand interaction and mixed sid | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-per-side-colors plus style/width  |
| border-radius shorthand one/two/three/four value expansion a | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-radius-corner-longhands, border-r |
| elliptical border-radius slash syntax and percentage radii | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-radius-percentage, border-radius- |
| border-radius overlap reduction/clamping when radii sums exc | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-radius-clamped; src/render/pdf.rs |
| border-radius applied consistently to borders, background cl | CSS Backgrounds & Borders 3 se | partial | src/render/pdf.rs normal background path uses per-corner/elliptical ra |
| border-image-source/slice/width/outset/repeat and border-ima | CSS Backgrounds & Borders 3 se | unsupported | rg border-image in src/ found no parser/computed/render support; no ta |
| outline and outline-offset in static output | CSS Basic UI / target backgrou | supported-tested | manifest backgrounds-borders: outline-solid, outline-offset-negative;  |
| box-shadow outer shadows with offsets, color, currentColor,  | CSS Backgrounds & Borders 3 se | supported-tested | manifest backgrounds-borders: border-box-shadow-offset, border-x-box-s |
| box-shadow blur radius, spread distance, negative spread, an | CSS Backgrounds & Borders 3 se | supported-tested | tests/parity/manifest/effects.json covers blur/spread/inset/negative-s |
| box-shadow with nonuniform, percentage, or elliptical border | CSS Backgrounds & Borders 3 se | partial | effects manifest covers simple border-radius shadow only; src/render/p |
| linear-gradient() directions: default, angles, to-side, to-c | CSS Images 3 section 3.4 linea | supported-tested | manifest backgrounds-borders: background-linear-gradient, background-l |
| linear-gradient() multiple color stops and hard stops with p | CSS Images 3 sections 3.4.2-3. | supported-tested | manifest backgrounds-gradients: linear-gradient-multi-stop, linear-gra |
| repeating-linear-gradient() with percentage stops | CSS Images 3 section 3.4 repea | supported-tested | manifest backgrounds-gradients: repeating-linear-gradient; src/style/c |
| gradient color-stop positions expressed as lengths, includin | CSS Images 3 section 3.4.3 col | unsupported | src/style/computed.rs parse_gradient_stops accepts percent tokens but  |
| gradient color interpolation hints/midpoints between stops | CSS Images 3 section 3.4.3 col | unsupported | src/style/computed.rs parse_gradient_stops expects each comma item to  |
| gradient alpha/transparent stop compositing over underlying  | CSS Images 3 gradient color st | partial | src/style/computed.rs parse_gradient_color ignores rgba alpha and src/ |
| radial-gradient() circle/ellipse shapes and default center | CSS Images 3 section 3.5 radia | supported-tested | manifest backgrounds-borders: background-radial-gradient, background-r |
| radial-gradient() size keywords closest-side, farthest-side, | CSS Images 3 section 3.5.1 rad | supported-untested | manifest backgrounds-gradients covers closest-side and farthest-side;  |
| radial-gradient() explicit radii and at-position syntax | CSS Images 3 sections 3.5.1-3. | supported-tested | manifest backgrounds-gradients: radial-gradient-sized-px, radial-gradi |
| repeating-radial-gradient() | CSS Images 3 section 3.5 repea | supported-tested | manifest backgrounds-gradients: repeating-radial-gradient; src/style/c |
| conic-gradient() basic, from angle, at position, and hard st | CSS Images 4 conic gradients;  | supported-tested | manifest backgrounds-borders: background-conic-gradient; manifest back |
| repeating-conic-gradient() | CSS Images 4 conic gradients;  | supported-tested | manifest backgrounds-gradients: repeating-conic-gradient; src/render/p |
| conic-gradient() color hints/midpoints and non-angle stop fi | CSS Images 4 conic gradients p | unsupported | src/style/computed.rs parse_conic_stops accepts colors plus angular/pe |
| CSS image() and element() functions as backgrounds | CSS Images 4 image functions | na-not-pdf-relevant | image() and element() remain Level 4 / not broadly stable in the consu |

### filters-effects-clip-color
_Existing parity coverage is strongest for sRGB color syntax, basic opacity numeric/clamp cases, display/visibility, simple box-shadow variants, several gradient mask-image forms, -webkit-mask-image aliasing, simple clip-path shapes, and basic image/filter cases. The main blind spots are group semantics and operation order: filters are not proved on text/descendant source graphics, filter chains can be order-insensitive, clip-path fixtures do not exercise geometry boxes or fill-rule, mask fixtures do not cover multiple layers/composite/positioning, blending covers only multiply/screen plus one background case, non-separable blend modes are absent, and CSS Color 4 wide-gamut/perceptual functions plus opacity percentages are untested or unsupported._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| filter property: none and ordered <filter-value-list> | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-chained; src/style/computed |
| filter creates an atomic filtered group of the element and d | Filter Effects 1 | partial | src/layout/images.rs apply_color_filters_to_box recolors only box back |
| filter painting can extend outside the border box without ch | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-blur-box and filter-on-box- |
| filter compositing order: filter before clipping, masking, a | Filter Effects 1 | supported-untested | src/render/pdf.rs wraps opacity/clip/mask around rendered boxes, but n |
| filter function blur() on raster images | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-blur-img; src/layout/images |
| filter function blur() on CSS boxes | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-blur-box and filter-on-box- |
| filter function blur() on text glyphs | Filter Effects 1 | partial | No manifest text blur filter fixture; source filtering is box/image-or |
| filter function brightness() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-brightness is expected_supp |
| filter function contrast() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-contrast expected unsupport |
| filter function grayscale() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-grayscale expected unsuppor |
| filter function sepia() | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-sepia expected implemented; |
| filter function saturate() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-saturate expected unsupport |
| filter function hue-rotate() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-hue-rotate expected unsuppo |
| filter function invert() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-invert expected unsupported |
| filter function opacity() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-opacity-fn expected unsuppo |
| filter function drop-shadow() basic image alpha shadow | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-drop-shadow expected implem |
| filter function drop-shadow() default color from currentColo | Filter Effects 1 | unsupported | src/style/computed.rs parse_drop_shadow defaults missing color to blac |
| filter function drop-shadow() as one item in an ordered filt | Filter Effects 1 | partial | src/style/computed.rs keeps a single drop_shadow field and src/layout/ |
| filter url(#id) referencing SVG filter: feColorMatrix subset | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-url-svg expected unsupporte |
| filter url(#id) referencing SVG filter primitives such as fe | Filter Effects 1 | unsupported | src/parser/svg.rs filter_element_color_ops has no general SVG filter p |
| color-interpolation-filters property for SVG filter primitiv | Filter Effects 1 | unsupported | No source handling found for color-interpolation-filters; CSS filter f |
| filter animation/interpolation rules | Filter Effects 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output; only the |
| filter hit testing and pointer behavior | Filter Effects 1 | na-not-pdf-relevant | Hit-testing behavior has no static PDF raster output |
| box-shadow offset and color | CSS Backgrounds / Compositing  | supported-tested | tests/parity/manifest/effects.json: box-shadow-offset and box-shadow-c |
| box-shadow blur radius | CSS Backgrounds / visual effec | partial | tests/parity/manifest/effects.json: box-shadow-blur marked partial; sr |
| box-shadow positive spread | CSS Backgrounds / visual effec | partial | tests/parity/manifest/effects.json: box-shadow-spread and box-shadow-s |
| box-shadow negative spread | CSS Backgrounds / visual effec | supported-tested | tests/parity/manifest/effects.json: box-shadow-negative-spread expecte |
| box-shadow inset shadows | CSS Backgrounds / visual effec | partial | tests/parity/manifest/effects.json: box-shadow-inset partial and box-s |
| box-shadow multiple shadows and paint order | CSS Backgrounds / visual effec | partial | tests/parity/manifest/effects.json: box-shadow-multiple partial; sourc |
| box-shadow currentColor | CSS Color 4 / CSS Backgrounds | supported-tested | tests/parity/manifest/effects.json: box-shadow-currentcolor; src/style |
| box-shadow with border-radius | CSS Backgrounds / visual effec | supported-tested | tests/parity/manifest/effects.json: box-shadow-border-radius |
| text-shadow offset shadows | CSS Text Decoration / visual e | partial | tests/parity/manifest/effects.json: text-shadow-offset expected unsupp |
| text-shadow blur | CSS Text Decoration / visual e | partial | tests/parity/manifest/effects.json: text-shadow-blur expected unsuppor |
| text-shadow multiple shadows and currentColor | CSS Text Decoration / CSS Colo | supported-untested | src/style/computed.rs parses text_shadow as a list and resolves curren |
| mix-blend-mode: normal | Compositing and Blending 1 | supported-tested | Default rendering and BlendMode::Normal in src/style/computed.rs |
| mix-blend-mode: multiply | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: mix-blend-mode-multiply expected u |
| mix-blend-mode: screen | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: mix-blend-mode-screen expected uns |
| mix-blend-mode separable values overlay, darken, lighten, co | Compositing and Blending 1 | supported-untested | src/style/computed.rs BlendMode includes these values and src/render/p |
| mix-blend-mode non-separable values hue, saturation, color,  | Compositing and Blending 1 | unsupported | Compositing 1 defines these blend modes, but src/style/computed.rs Ble |
| mix-blend-mode establishes a blended stacking context for te | Compositing and Blending 1 | partial | src/render/pdf.rs wraps container branches in blend states; top-level  |
| background-blend-mode: multiply | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: background-blend-mode-multiply exp |
| background-blend-mode other separable values | Compositing and Blending 1 | supported-untested | src/style/computed.rs BlendMode supports separable PDF blend names; no |
| background-blend-mode non-separable values hue, saturation,  | Compositing and Blending 1 | unsupported | src/style/computed.rs BlendMode lacks non-separable blend modes |
| background-blend-mode list matching multiple background laye | Compositing and Blending 1 | unsupported | ComputedStyle has a single background_blend_mode field; parser does no |
| isolation property: auto and isolate | Compositing and Blending 1 | unsupported | No source handling found for isolation; important for constraining ble |
| compositing animation/interpolation | Compositing and Blending 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output |
| clip-path: inset() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-inset expected unsuppo |
| clip-path: inset() round corners | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-inset-round expected u |
| clip-path: circle() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-circle expected unsupp |
| clip-path: ellipse() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-ellipse expected unsup |
| clip-path: polygon() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-polygon expected unsup |
| clip-path basic shapes with reference geometry boxes border- | CSS Masking 1 | unsupported | src/style/computed.rs parse_clip_path does not parse shape-box or geom |
| clip-path polygon() fill-rule nonzero/evenodd | CSS Masking 1 / CSS Shapes | unsupported | src/style/computed.rs parse_clip_path does not parse polygon fill-rule |
| clip-path url(#clipPath) SVG clip source | CSS Masking 1 | unsupported | No clip-path url handling found; parser handles only circle/ellipse/in |
| clip-rule property for referenced SVG clipPath | CSS Masking 1 | unsupported | No source handling found for clip-rule; relevant when clip-path refere |
| deprecated clip: rect(...) on absolutely positioned elements | CSS Masking 1 | unsupported | Spec requires support, but no parser/render handling found for the cli |
| mask-image: none | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-none |
| mask-image linear-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-linear-gradient and m |
| mask-image radial-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-radial-gradient |
| mask-image conic-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-conic-gradient |
| mask-image repeating-linear-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-repeating-linear |
| mask-image repeating-radial-gradient() and repeating-conic-g | CSS Masking 1 | partial | src/render/pdf.rs rasterize_mask_coverage handles repeating linear/rad |
| mask-image url() SVG/image masks | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: mask-image-url-svg expected unsu |
| -webkit-mask-image alias for compatibility | CSS Masking 1 / browser-shippe | supported-tested | tests/parity/manifest/clip-mask.json: webkit-mask-image-alias |
| mask-mode: alpha, luminance, match-source | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: mask-mode-luminance; src/style/c |
| mask-position, mask-repeat, mask-size | CSS Masking 1 | unsupported | Parser may retain raw CssValue, but ComputedStyle/render mask path has |
| mask-origin and mask-clip | CSS Masking 1 | unsupported | No render-time geometry mapping found for mask-origin or mask-clip |
| multiple mask layers | CSS Masking 1 | unsupported | src/style/computed.rs parse_mask_image keeps a single MaskSource and r |
| mask-composite: add, subtract, intersect, exclude | CSS Masking 1 | unsupported | No ComputedStyle field or render path applying mask-composite between  |
| mask shorthand full grammar | CSS Masking 1 | partial | src/style/computed.rs maps mask through parse_mask_image only; positio |
| mask-type on SVG <mask>: luminance and alpha | CSS Masking 1 | partial | src/render/pdf.rs includes SVG alpha/luminance conversion logic, but n |
| mask-border-* family | CSS Masking 1 | unsupported | No source handling found for mask-border-source/slice/width/outset/rep |
| mask animation/interpolation | CSS Masking 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output |
| CSS named colors including CSS Color 4 named set | CSS Color 4 | partial | tests/parity/manifest/color-opacity.json tests navy/rebeccapurple/case |
| transparent keyword | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-transparent |
| currentColor keyword in dependent colors | CSS Color 4 | partial | tests/parity/manifest/color-opacity.json: color-currentcolor-border an |
| hex colors #rgb and #rrggbb | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hex-short and color-he |
| hex colors with alpha #rgba and #rrggbbaa | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hex-alpha-short and co |
| legacy rgb()/rgba() comma syntax | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-rgb and color-rgba-alp |
| modern rgb() space/slash syntax, percentages, none component | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-rgb-modern-slash, colo |
| alpha values in color functions as number or percentage | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-alpha-percentage and r |
| hsl()/hsla() legacy and modern syntax, hue angles, powerless | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hsl, color-hsla, color |
| hwb() | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hwb and color-hwb-gray |
| lab() and lch() | CSS Color 4 | unsupported | No manifest fixture or source parser support found for lab()/lch() CSS |
| oklab() and oklch() | CSS Color 4 | unsupported | No manifest fixture or source parser support found for oklab()/oklch() |
| color() function with srgb and srgb-linear | CSS Color 4 | unsupported | No manifest fixture or source parser support found for color(<predefin |
| color() function with display-p3, a98-rgb, prophoto-rgb, rec | CSS Color 4 | unsupported | No manifest fixture or source parser support found for wide-gamut or X |
| opacity property with numeric values and clamping | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: opacity-half, opacity-clamp, |
| opacity property with percentage values | CSS Color 4 | unsupported | CSS Color 4 allows percentages; src/style/computed.rs opacity assignme |
| opacity group flattening for nested boxes | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: opacity-nested-group |
| opacity group flattening for overlapping text glyphs | CSS Color 4 | supported-untested | No manifest fixture forces overlap within one translucent text element |
| visibility: hidden, collapse, and visible descendants | CSS Display / Color visibility | supported-tested | tests/parity/manifest/color-opacity.json: visibility-hidden, visibilit |
| display: none | CSS Display | supported-tested | tests/parity/manifest/color-opacity.json: display-none |
| text glyph color from color property | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-text-glyph |
| system colors and forced-color dynamic adaptation | CSS Color 4 | na-not-pdf-relevant | Static PDF comparison should not depend on interactive forced-color UA |

### flexbox
_The existing flexbox manifest is strong for core row/column flex layout: display:flex, physical directions and reverse directions, nowrap/wrap/wrap-reverse, flex-flow, order including negative ties, justify-content and align-items basics, align-content for wrapped rows, grow/shrink/basis including fractional sums and clamps, fixed and percentage gaps, main-axis auto margins, nested flex containers, min/max clamps, and percent-height stretch. Blind spots remain around unsupported parse surface, generated anonymous items, logical axis mapping, safe overflow alignment, synthesized baselines, abspos static-position behavior, flexed aspect-ratio sizing, column-wrap cross distribution, auto-min escape hatches, one-sided cross auto margins, intrinsic flex-basis keywords, and paged fragmentation._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| display:flex creates a block-level flex container | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-display-flex; src/style/computed.rs:3170-3178 pars |
| display:inline-flex creates an inline-level flex container | https://www.w3.org/TR/css-flex | unsupported | no manifest entry; src/style/computed.rs:3170-3178 parses flex but not |
| flex item generation for element children | https://www.w3.org/TR/css-flex | supported-tested | many manifest entries use element children; src/layout/flex.rs:170-219 |
| anonymous flex items from direct text children | https://www.w3.org/TR/css-flex | unsupported | no manifest entry; src/layout/flex.rs:170-179 filters only DomNode::El |
| absolutely positioned flex children are out of flex flow and | https://www.w3.org/TR/css-flex | partial | no flexbox manifest entry; src/layout/flex.rs:183-291 excludes abspos  |
| flex-direction row, row-reverse, column, column-reverse | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-flex-direction-column, flexbox-flex-direction-row |
| main-axis mapping through direction and writing-mode | https://www.w3.org/TR/css-flex | partial | no manifest entry; src/layout/flex.rs:1338-1380 treats row as physical |
| flex-wrap nowrap, wrap, wrap-reverse for rows | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-flex-wrap-nowrap, flexbox-flex-wrap, flexbox-flex |
| flex-direction:column with flex-wrap creates additional colu | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-column-wrap; src/layout/flex.rs:1380-1468 has colu |
| flex-flow shorthand combines direction and wrap | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-flex-flow; src/style/computed.rs:3182-3191 parses  |
| order property reorders layout and paint order, including ne | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-order and flexbox-order-negative; src/layout/flex |
| static flex item z-index creates flex painting order effect | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-z-index-static; src/render/pdf.rs keeps FlexCell z |
| flex-grow positive free-space distribution | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-flex-grow, flexbox-flex-grow-basis-auto; src/layo |
| flex-grow factors whose sum is less than one | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-grow-fractional; src/layout/flex.rs:2022-2033 caps |
| flex-grow max-main clamp and freeze redistribution | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-max-width-clamp; src/layout/flex.rs:2045-2067 free |
| flex-shrink scaled shrink factors and negative free-space di | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-flex-shrink and flexbox-flex-shrink-zero; src/lay |
| flex-shrink factors whose sum is less than one | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-shrink-fractional; src/layout/flex.rs:2081-2089 ab |
| flex-shrink min-main clamp and freeze redistribution | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-min-width-constraint, flexbox-min-height-column;  |
| flex-basis length, auto, content, zero, and percentage | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-flex-basis, flexbox-flex-basis-auto, flexbox-basi |
| flex-basis intrinsic sizing keywords min-content, max-conten | https://www.w3.org/TR/css-flex | unsupported | no manifest entry; src/style/computed.rs:3287-3321 handles auto/conten |
| flex shorthand keywords and numeric forms | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-flex-shorthand-keywords; src/style/computed.rs:332 |
| automatic minimum main size content floor for min-width:auto | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-min-content-no-overflow; src/layout/flex.rs:1094-1 |
| automatic minimum size escape hatches: min-width:0 or non-vi | https://www.w3.org/TR/css-flex | supported-untested | no flexbox manifest entry; src/layout/flex.rs:1094-1118 only applies a |
| min/max main-axis and cross-axis clamps | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-max-width-clamp, flexbox-min-height-column, flexb |
| justify-content flex-start, flex-end, center, space-between, | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-justify-content-*; src/layout/flex.rs:2328-2363 d |
| justify-content start/end/left/right aliases from CSS Box Al | https://www.w3.org/TR/css-alig | partial | no manifest entry; src/style/computed.rs:3215-3219 parses aliases, but |
| justify-content safe and unsafe overflow-position prefixes | https://www.w3.org/TR/css-alig | partial | manifest id flexbox-justify-safe-center only covers fitting content; s |
| align-items flex-start, flex-end, center, stretch, baseline | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-align-items-flex-start, flexbox-align-items-flex- |
| baseline synthesis for flex items without text baselines | https://www.w3.org/TR/css-flex | partial | no manifest entry; src/render/pdf.rs baseline path says no text baseli |
| align-self auto, flex-start, flex-end, center, baseline, str | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-align-self-flex-end, flexbox-align-self-center, f |
| align-content flex-start, flex-end, center, space-between, s | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-align-content-*; src/layout/flex.rs:1732-1807 dis |
| align-content distribution for column-wrap flex containers | https://www.w3.org/TR/css-flex | supported-untested | manifest id flexbox-column-wrap covers wrapping but not align-content  |
| main-axis auto margins absorb positive free space before jus | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-margin-auto-main-end, flexbox-margin-auto-split;  |
| cross-axis auto margins override align-self and suppress str | https://www.w3.org/TR/css-flex | supported-untested | manifest id flexbox-margin-auto-center covers both-axis auto but not o |
| fixed flex item margins do not collapse and participate in p | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-item-margin; src/layout/flex.rs:673-683 maps fixed |
| percentage margins and paddings on flex items resolve agains | https://www.w3.org/TR/css-flex | supported-tested | covered indirectly by units-values percent fixtures plus flex item mar |
| gap, row-gap, column-gap fixed lengths, two-value gap, and p | https://www.w3.org/TR/css-alig | supported-tested | manifest ids flexbox-gap, flexbox-gap-two-value, flexbox-row-column-ga |
| nested flex containers as flex items | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-nested-flex, flexbox-nested-row-in-row, flexbox-n |
| percentage-height descendants resolve after align-items:stre | https://www.w3.org/TR/css-flex | supported-tested | manifest id flexbox-percent-height-stretch; src/layout/flex.rs:1554-16 |
| percentage flex-basis in a definite-height column flex conta | https://www.w3.org/TR/css-flex | supported-untested | no manifest entry; src/layout/flex.rs:1264-1296 resolves column percen |
| aspect-ratio flex items with flexible main sizes | https://www.w3.org/TR/css-flex | partial | no manifest entry; src/layout/flex.rs computes aspect-ratio height bef |
| subpixel and fractional free-space distribution/rounding | https://www.w3.org/TR/css-flex | supported-tested | manifest ids flexbox-grow-fractional and flexbox-shrink-fractional cov |
| fragmenting multi-line flex layout across pages | https://www.w3.org/TR/css-flex | unsupported | no manifest entry; src/layout/paginate.rs:125-139 and 1603-1614 estima |

### fragmentation-paged
_The current target manifests already cover the core happy paths for page breaks, break-inside avoidance, widows/orphans, basic named-page geometry, first/left/right page margins, page/root backgrounds, basic footnotes, basic running elements, table-row splitting, multicol count/width/gap/rule/span/fill, and multicol pagination. The blind spots are concentrated in boundary-order semantics, side/selected page margin boxes, :blank pages, table header/footer repetition, flex/grid fragmentation, column-specific breaks and percentage gaps, column-rule suppression beside empty columns, and deeper GCPM behavior._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| page fragmentainers and pagination of normal-flow block cont | CSS Fragmentation Level 3; CSS | supported-tested | paged-media manifest: paged-forced-break-two-pages, break-before-page- |
| break-before/break-after: page and legacy page-break-before/ | CSS Fragmentation Level 3 §4.2 | supported-tested | paged-media manifest: break-before-page-modern-real, break-after-page- |
| break-before/break-after: left, right, recto, verso parity b | CSS Fragmentation Level 3 §4.2 | supported-untested | src/style/computed.rs BreakValue supports Left/Right/Recto/Verso and s |
| forced break precedence when break-after and break-before me | CSS Fragmentation Level 3 §4.3 | partial | src/layout/engine.rs emits separate LayoutElement::PageBreak entries f |
| break-before/break-after: avoid and avoid-page suppression o | CSS Fragmentation Level 3 §4.4 | partial | src/style/computed.rs parses avoid/avoid-page, but source evidence onl |
| break-before/break-after: column and avoid-column in multico | CSS Fragmentation Level 3 §4.2 | unsupported | src/style/computed.rs BreakValue::from_keyword lacks column and avoid- |
| break-inside: avoid and avoid-page for page fragmentation of | CSS Fragmentation Level 3 §4.4 | supported-tested | paged-media manifest: paged-break-inside-avoid, break-inside-avoid-rea |
| break-inside: avoid-column inside multicol with column-fill: | CSS Fragmentation Level 3 §4.4 | partial | src/style/computed.rs collapses avoid-column into break_inside_avoid,  |
| legacy page-break-inside: avoid alias | CSS Fragmentation Level 3 §3.4 | supported-tested | paged-media manifest: page-break-inside-avoid-table-straddle and page- |
| orphans and widows line-count constraints | CSS Fragmentation Level 3 §3.3 | supported-tested | paged-media manifest: orphans-widows-default, orphans-3, orphans-4, wi |
| box-decoration-break: slice and clone at page fragment bound | CSS Fragmentation Level 3 §5.4 | supported-tested | src/style/computed.rs parses BoxDecorationBreak::Slice/Clone and src/l |
| fragmented margins, borders, backgrounds and padding on spli | CSS Fragmentation Level 3 §5 | supported-tested | src/layout/paginate.rs split_container and split_text_block create fir |
| fragmenting raster images/replaced boxes across pages | CSS Fragmentation Level 3 §5.1 | partial | src/layout/paginate.rs split_image_block only splits object-fit:fill i |
| fragmenting table rows/cells taller than a page | CSS Fragmentation Level 3 §5.1 | supported-tested | paged-media manifest: table-row-taller-than-page; src/layout/paginate. |
| repeating table header and footer groups on each page fragme | CSS Fragmentation Level 3 tabl | supported-untested | src/layout/paginate.rs tracks pending_table_headers and pending_table_ |
| fragmenting flex containers and flex items across pages | CSS Fragmentation Level 3 §5.1 | partial | src/layout/paginate.rs split_element does not split LayoutElement::Fle |
| fragmenting grid containers and grid items across pages | CSS Fragmentation Level 3 §5.1 | partial | src/layout/paginate.rs split_element does not split LayoutElement::Gri |
| @page size descriptor with explicit width/height lengths | CSS Paged Media Level 3 §7.1 | supported-tested | all target fixtures use @page{size:<W>px <H>px}; src/parser/css/page.r |
| @page size descriptor common page-size keywords and orientat | CSS Paged Media Level 3 §7.1 | partial | src/parser/css/page.rs supports A3/A4/A5/B5/letter/legal and orientati |
| @page margin shorthand and margin-* longhands | CSS Paged Media Level 3 §6 and | partial | src/parser/css/page.rs supports one-, two-, and four-value margin shor |
| @page background painting over the page box/bleed area | CSS Paged Media Level 3 §3.2 a | supported-tested | paged-media manifest: at-page-background-bleed; src/parser/css/page.rs |
| root/canvas background behavior in paged output | CSS Paged Media Level 3 §3 and | supported-tested | paged-media manifest: root-background-content-box |
| @page :first selector | CSS Paged Media Level 3 §4.2.1 | supported-tested | paged-media manifest: paged-first-page-margin; src/parser/css/page.rs  |
| @page :left and :right spread selectors | CSS Paged Media Level 3 §4.2.1 | supported-tested | paged-media manifest: paged-spread-left-right-margins; src/parser/css/ |
| @page :blank selector for blank pages inserted by forced bre | CSS Paged Media Level 3 §4.2.3 | unsupported | src/parser/css/page.rs recognizes PageSelector::Blank, but src/lib.rs  |
| named pages via the page property, including named page marg | CSS Paged Media Level 3 §8 | supported-tested | paged-media manifest: paged-named-page, paged-named-page-margin, paged |
| page selector lists and named-page plus pseudo-page combinat | CSS Paged Media Level 3 §4.3 p | unsupported | src/parser/css/page.rs classify_page_selector returns one PageSelector |
| page margin boxes in top and bottom bands with literal conte | CSS Paged Media Level 3 §5.2 p | partial | src/parser/css/page.rs recognizes all margin box names but src/render/ |
| page margin boxes in left/right side bands | CSS Paged Media Level 3 §5.2 p | unsupported | src/parser/css/page.rs parses @left-* and @right-* margin boxes, but s |
| selected and named @page margin boxes | CSS Paged Media Level 3 §5.1 a | unsupported | src/lib.rs collects margin boxes only from page rules with selector == |
| page counters counter(page) and counter(pages) in page-margi | CSS Paged Media Level 3 §6.1 c | supported-tested | src/parser/css/page.rs parse_margin_box_content handles counter(page)  |
| counter-reset/counter-increment for page counters in @page | CSS Paged Media Level 3 §6.1 | unsupported | source searches show page margin box rendering uses raw page_num/total |
| column-count, column-width and columns shorthand column coun | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-count-three, multicol-column-width, |
| column-gap: normal and fixed lengths | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-gap, multicol-column-gap-normal; sr |
| column-gap percentages | CSS Multi-column Layout Level  | unsupported | src/style/computed.rs stores column_gap_pct, but src/layout/multicol.r |
| column-rule width/style/color and shorthand | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-rule-solid, multicol-column-rule-lo |
| column rules are drawn only between adjacent columns that bo | CSS Multi-column Layout Level  | partial | src/layout/multicol.rs emits rule spans for column gaps based on colum |
| column-span: all spanners interrupt multicol flow | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-span-all; paged-media manifest: mul |
| column-fill: balance | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-fill-balance and several default ba |
| column-fill: auto with definite height and pagination | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-column-fill-auto, multicol-page-break-flow |
| multicol pagination across page fragments with rules/gaps pr | CSS Multi-column Layout Level  | supported-tested | multicol manifest: multicol-page-break-flow, multicol-three-cols-page- |
| column balancing with spanners and page boundaries | CSS Multi-column Layout Level  | supported-tested | paged-media manifest: multicol-span-all-page-break; multicol manifest: |
| float: footnote and basic footnote area placement | CSS Generated Content for Page | supported-tested | paged-media manifest: footnote-float with oracle weasyprint; src/style |
| @footnote styling, ::footnote-call and ::footnote-marker | CSS Generated Content for Page | partial | basic float:footnote exists, but source searches do not show @footnote |
| footnote-display: block, inline, compact | CSS Generated Content for Page | unsupported | source searches show no footnote-display parser or layout branch; no t |
| footnote-policy: auto, line, block | CSS Generated Content for Page | unsupported | source searches show no footnote-policy parser or pagination behavior; |
| running elements with position: running(name) and content: e | CSS Generated Content for Page | supported-tested | paged-media manifest: running-element-header with oracle weasyprint; s |
| element(name, first/start/last/first-except) running-element | CSS Generated Content for Page | unsupported | src/parser/css/page.rs parse_margin_box_content only handles element(n |
| named strings: string-set and string() in page-margin boxes | CSS Generated Content for Page | unsupported | source searches found no string-set support; src/parser/css/page.rs pa |
| running headers sourced from selected or named pages | CSS Paged Media Level 3 page-m | partial | running elements can render in unselected @page margin boxes, but src/ |

### grid
_Current grid parity coverage is strong for basic display:grid, fixed/percent/fr columns, fixed rows, integer repeat(), simple minmax(.,1fr), single auto columns, row/column gap lengths, default row auto-placement, grid-auto-flow:column, numeric spans, positive line placement, named lines, named areas, dot cells, and three basic item-alignment cases. It is thin or blind for intrinsic/fit-content track sizing, auto-repeat, implicit columns, grid shorthands/longhands, order and overlap painting, subgrid, alignment distribution/self/baseline edge cases, and grid fragmentation._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| display:grid establishes a block-level grid formatting conte | CSS Grid 1 section 2, grid con | supported-tested | manifest grid-display-grid; src/style/computed.rs:3177 maps display:gr |
| display:inline-grid establishes an inline-level grid contain | CSS Grid 1 section 2, grid con | unsupported | src/style/computed.rs:27-33 Display has Grid but no InlineGrid; no man |
| grid items are laid out into grid cells and blockified as gr | CSS Grid 1 sections 6 and 10 | partial | src/layout/grid.rs:928-997 filters element children and absolute child |
| fixed <length> track sizing for columns | CSS Grid 1 section 7.2 track s | supported-tested | manifest grid-display-grid, grid-template-columns-repeat; src/style/co |
| fixed <length> track sizing for rows | CSS Grid 1 section 7.2 track s | supported-tested | manifest grid-template-rows; src/layout/grid.rs:1051-1059 applies fixe |
| percentage track sizing in grid-template-columns | CSS Grid 1 section 7.2 track s | supported-tested | manifest grid-template-columns-percent; src/style/computed.rs:6872-687 |
| fr flexible track sizing in columns | CSS Grid 1 sections 7.2 and 12 | supported-tested | manifest grid-template-columns-fr-mix; src/layout/grid.rs:80-178 imple |
| fr flexible track sizing in rows | CSS Grid 1 sections 7.2 and 12 | partial | src/layout/grid.rs:214-221 says fr/auto/minmax rows fall back to auto  |
| auto track sizing for columns | CSS Grid 1 sections 7.2 and 12 | partial | manifest grid-template-columns-auto covers leftover auto; src/layout/g |
| auto rows and default row sizing | CSS Grid 1 sections 7.2, 7.6,  | partial | src/layout/grid.rs:1051-1111 uses fixed/grid-auto/content fallback and |
| minmax(min,max) with flexible max | CSS Grid 1 section 7.2.1 minma | supported-tested | manifest grid-template-columns-minmax; src/style/computed.rs:6885-6914 |
| minmax() with fixed max caps and non-flex max sizing | CSS Grid 1 section 7.2.1 minma | partial | src/style/computed.rs:6902-6913 stores fixed max, but src/layout/grid. |
| minmax(auto,...) and minmax(...,auto) intrinsic semantics | CSS Grid 1 section 7.2.1 minma | partial | src/style/computed.rs:6892-6904 coerces auto min to 0 and auto max to  |
| min-content track sizing keyword | CSS Grid 1 section 7.2.1 track | partial | src/style/computed.rs:7029-7031 approximates min-content as GridTrack: |
| max-content track sizing keyword | CSS Grid 1 section 7.2.1 track | partial | src/style/computed.rs:7029-7031 approximates max-content as GridTrack: |
| fit-content(<length-percentage>) track sizing function | CSS Grid 1 section 7.2.1 fit-c | partial | src/style/computed.rs:7014-7019 parses fit-content() as Auto and expli |
| repeat(<integer>, <track-list>) fixed repeat notation | CSS Grid 1 section 7.2.3 repea | supported-tested | manifest grid-template-columns-repeat; src/style/computed.rs:6963-6999 |
| repeat() with multiple tracks in the repeated pattern | CSS Grid 1 section 7.2.3 repea | supported-untested | src/style/computed.rs:6981-6995 recursively expands a track list patte |
| repeat() with bracketed line names across repetitions | CSS Grid 1 section 7.2.3 repea | partial | src/style/computed.rs:6981-6995 merges line names, but placement resol |
| repeat(auto-fill, ...) repeat-to-fill | CSS Grid 1 section 7.2.3.2 aut | partial | src/style/computed.rs:6974-6979 hard-codes auto-fill to 3 repetitions; |
| repeat(auto-fit, ...) repeat-to-fill with empty-track collap | CSS Grid 1 section 7.2.3.2 aut | partial | src/style/computed.rs:6974-6979 hard-codes auto-fit to 3 repetitions a |
| subgrid value for grid-template-columns/grid-template-rows | CSS Grid 2 sections 3.4 and 9  | unsupported | Grid 2 CRD and MDN Subgrid page fetched; src/style/computed.rs:6867-68 |
| subgrid line-name augmentation and subgrid repeat(auto-fill) | CSS Grid 2 sections 7.2.6.2 an | unsupported | no subgrid model in ComputedStyle fields at src/style/computed.rs:1421 |
| grid-template-areas named rectangular areas | CSS Grid 1 section 7.3 named a | supported-tested | manifest grid-template-areas-basic and grid-area-span-rows; src/style/ |
| grid-template-areas null cells using dot tokens | CSS Grid 1 section 7.3 named a | supported-tested | manifest grid-template-areas-dot; src/style/computed.rs:6834-6842 trea |
| grid-template-areas invalid non-rectangular/disconnected are | CSS Grid 1 section 7.3 named a | partial | src/style/computed.rs:6819-6864 pads rows and does not validate rectan |
| implicit line names generated by grid-template-areas (<area> | CSS Grid 1 section 7.3.2 impli | supported-untested | src/layout/grid.rs:541-566 derives area start/end lines; no manifest f |
| implicit named areas generated by explicit foo-start/foo-end | CSS Grid 1 section 7.3.3 impli | supported-untested | grid-area:<name> resolves through named <name>-start/end in src/layout |
| grid-template shorthand | CSS Grid 1 section 7.4 explici | unsupported | src/parser/css/values.rs:461 preserves grid-template but src/style/com |
| grid shorthand including auto-flow syntax and reset semantic | CSS Grid 1 section 7.8 grid sh | unsupported | no parser/computed handling for property grid in rg results; no manife |
| grid-auto-rows single fixed implicit row size | CSS Grid 1 section 7.6 implici | supported-tested | manifest grid-implicit-tracks; src/style/computed.rs:3445-3454 parses  |
| grid-auto-rows multiple-size repeating pattern | CSS Grid 1 section 7.6 implici | unsupported | ComputedStyle has grid_auto_rows: Option<f32> only at src/style/comput |
| grid-auto-columns implicit column sizing | CSS Grid 1 section 7.6 implici | unsupported | no grid-auto-columns field or parser handling; src/layout/grid.rs:1044 |
| implicit rows created by auto-placement overflow | CSS Grid 1 sections 7.5 and 8. | supported-tested | manifest grid-implicit-tracks; src/layout/grid.rs:696-725 grows occupa |
| implicit columns created by placement outside explicit grid | CSS Grid 1 sections 7.5 and 8. | partial | src/layout/grid.rs:841 updates max_cols, but src/layout/grid.rs:1044-1 |
| grid-auto-flow: row sparse auto-placement | CSS Grid 1 section 8.5 auto-pl | supported-tested | manifest grid-display-grid and span fixtures exercise default row flow |
| grid-auto-flow: column auto-placement | CSS Grid 1 sections 7.7 and 8. | supported-tested | manifest grid-auto-flow-column; src/layout/grid.rs:758-807 implements  |
| grid-auto-flow: dense backfilling | CSS Grid 1 sections 7.7 and 8. | supported-untested | src/style/computed.rs:3455-3458 parses dense; src/layout/grid.rs:748-8 |
| auto-placement of items with definite row or definite column | CSS Grid 1 section 8.5 auto-pl | partial | src/layout/grid.rs:681-693 resolves axes independently and src/layout/ |
| order property affects grid auto-placement and painting orde | CSS Grid 1 sections 6.3 and 8  | unsupported | ComputedStyle stores order at src/style/computed.rs:1384-1386, but src |
| grid-column and grid-row shorthands | CSS Grid 1 section 8.4 placeme | supported-tested | manifest grid-column-span, grid-row-span, grid-column-line-numbers, gr |
| grid-column-start/grid-column-end/grid-row-start/grid-row-en | CSS Grid 1 section 8.3 line-ba | unsupported | src/parser/css/values.rs does not preserve grid-column-start/end or gr |
| positive integer line placement | CSS Grid 1 section 8.3 line-ba | supported-tested | manifest grid-column-line-numbers and grid-row-line-numbers; src/layou |
| negative integer line placement from the end edge | CSS Grid 1 section 8.3 line-ba | supported-untested | src/layout/grid.rs:583-586 resolves negative line numbers; no parity f |
| named line placement by explicit bracketed line names | CSS Grid 1 section 8.3 line-ba | supported-tested | manifest grid-named-lines-basic and grid-named-line-placement; src/sty |
| repeated named lines with nth occurrence syntax (<integer> & | CSS Grid 1 section 8.3 grid-li | unsupported | src/style/computed.rs:6761-6783 parse_grid_line accepts either integer |
| numeric span placement (span N) | CSS Grid 1 section 8.3 grid-li | supported-tested | manifest grid-column-span, grid-row-span, grid-column-span-to-line; sr |
| named span placement (span <custom-ident>) | CSS Grid 1 section 8.3 grid-li | partial | src/style/computed.rs:6771-6772 parses SpanNamed but src/layout/grid.r |
| grid-area single named area placement | CSS Grid 1 section 8.4 grid-ar | supported-tested | manifest grid-template-areas-basic, grid-template-areas-dot, grid-area |
| grid-area four-line form row-start/col-start/row-end/col-end | CSS Grid 1 section 8.4 grid-ar | supported-tested | manifest grid-area-line-form; src/style/computed.rs:6798-6809 parses t |
| overlapping grid areas and z-index stacking for static grid  | CSS Grid 1 section 6.5 z-axis  | unsupported | src/layout/grid.rs:1163-1170 skips later overlapping cells; TableCell  |
| absolutely positioned children of a grid container are not g | CSS Grid 1 section 10 absolute | supported-untested | src/layout/grid.rs:982-997 filters absolute children out of grid items |
| row-gap, column-gap, and gap on grids | CSS Grid 1 section 10.1 gutter | supported-tested | manifest grid-gap; src/layout/grid.rs:921-922 reads row_gap/column_gap |
| legacy grid-gap/grid-row-gap/grid-column-gap aliases | CSS Align 3 gutter aliases ref | supported-untested | src/parser/css/inline.rs:632-635 and src/parser/css/values.rs:419-423  |
| percentage gaps in grid layout | CSS Align 3 gap percentage beh | partial | src/style/computed.rs has parser unit grid_gap_from_percentage but gri |
| justify-items item alignment start/end/center/stretch | CSS Grid 1 section 10.3 inline | supported-tested | manifest grid-justify-items-end and grid-place-items-center; src/layou |
| align-items item alignment start/end/center/stretch | CSS Grid 1 section 10.4 block- | supported-tested | manifest grid-align-items-start and grid-place-items-center; src/style |
| place-items shorthand | CSS Grid 1 alignment sections  | supported-tested | manifest grid-place-items-center; src/style/computed.rs:3462-3470 pars |
| justify-self/align-self/place-self on individual grid items | CSS Grid 1 sections 10.3 and 1 | supported-untested | src/style/computed.rs:3472-3499 parses self-alignment; src/layout/grid |
| baseline alignment for grid items | CSS Grid 1 section 10 alignmen | unsupported | GridAlign enum at src/style/computed.rs:157-169 has only stretch/start |
| justify-content and align-content distribution of the grid w | CSS Grid 1 section 10.5 aligni | partial | src/style/computed.rs:3199-3250 parses flex/content alignment fields,  |
| auto margins on grid items for alignment | CSS Grid 1 section 10.2 aligni | unsupported | ComputedStyle tracks auto margins for block/flex, but src/layout/grid. |
| fragmentation between grid rows in paged media | CSS Grid 1 section 13 fragment | supported-untested | grid rows are emitted as Container children at src/layout/grid.rs:1282 |
| fragmentation inside a grid item or across a spanned grid ar | CSS Grid 1 section 13 fragment | partial | src/layout/grid.rs:1147-1152 says multi-row items are approximated on  |
| CSSOM resolved value serialization of grid-template-* track  | CSS Grid 1 section 7.2.6 and G | na-not-pdf-relevant | CSSOM serialization has no visual effect in a static PDF without scrip |

### tables
_The current tables manifest strongly covers ordinary HTML table grids, positive rowspan/colspan, simple separated and collapsed borders, one/two-value border-spacing, fixed layout with colgroup/percentage/remainder cases, basic auto layout, td/th padding and alignment defaults, top/bottom captions, empty whitespace cells, baseline/top/middle/bottom vertical alignment, row-group ordering, and multipage header/footer repetition. The main blind spots are spec-level table fixup/display roles, collapsed-border conflict winners, subtle auto/fixed layout interactions, empty-cell visibility semantics, zero-rowspan, multiple captions, and combined-property edge cases._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| HTML table element establishes the table grid and table wrap | CSS Tables 3 §2.1, CSS 2.2 §17 | supported-tested | manifest tables-basic-grid; src/layout/engine.rs:2554-2567 dispatches  |
| CSS display: table and inline-table create table formatting  | MDN display table/internal val | unsupported | Display enum omits table/inline-table in src/style/computed.rs:25-34 a |
| CSS internal table display values: table-row-group, table-he | MDN display table/internal val | unsupported | Computed display has no internal table roles; layout dispatch is tag-b |
| Anonymous table fixup for missing parents/children around ta | CSS Tables 3 §2.2 and §2.2.1 | unsupported | src/layout/table.rs:608-670 only consumes actual tr/thead/tbody/tfoot  |
| Missing-cells fixup appends anonymous cells so every row cov | CSS Tables 3 §3.4 | partial | src/layout/table.rs:1384-1390 breaks when a short row has no next cell |
| Table row groups thead/tbody/tfoot participate in the row gr | CSS Tables 3 §3.2; CSS 2.2 §17 | supported-tested | manifest tables-thead-tbody-tfoot and multipage-tfoot-before-tbody; sr |
| Table headers and footers repeat across page breaks in paged | CSS Tables 3 table-header-grou | supported-tested | manifest multipage-thead-repeat, multipage-tfoot-repeat, multipage-thr |
| colgroup and col elements contribute column widths in fixed  | CSS 2.2 §17.5.2.1; CSS Tables  | supported-tested | manifest tables-layout-fixed; src/layout/table.rs:735-823 collects col |
| colgroup/col span attributes and bare colgroup columns | CSS Tables 3 §3.3 table-column | supported-untested | src/layout/table.rs:735-823 handles col span and bare colgroup span, b |
| HTML colspan distributes a cell across multiple columns | CSS Tables 3 §3.3.1 and §3.8 | supported-tested | manifest tables-colspan and tables-colspan-rowspan; src/layout/table.r |
| HTML rowspan distributes a cell across multiple rows | CSS Tables 3 §3.3.1 and §3.8 | supported-tested | manifest tables-rowspan, tables-rowspan-stagger, tables-colspan-rowspa |
| HTML rowspan=0 spans the remaining rows in the row group | HTML table model as referenced | unsupported | src/layout/table.rs:1510-1513 parses rowspan then clamps with max(1),  |
| table-layout: fixed uses table/column/first-row widths and i | CSS 2.2 §17.5.2.1; CSS Tables  | supported-tested | manifest tables-layout-fixed, tables-width-percent-columns, tables-lay |
| table-layout: fixed cell overflow is controlled by the cell  | CSS 2.2 §17.5.2.1 | partial | table cells carry content boxes but src/render/pdf/layout_elements.rs: |
| table-layout: auto computes min/preferred widths from non-sp | CSS 2.2 §17.5.2.2; CSS Tables  | supported-tested | manifest tables-layout-auto; src/layout/table.rs:1050-1160 computes mi |
| table-layout: auto may grow past a declared width when nowra | CSS 2.2 §17.5.2.2; CSS Tables  | partial | manifest tables-layout-auto-overflow is expected_support partial and d |
| table-layout: auto distributes spanning-cell min/max contrib | CSS Tables 3 §3.8.3; CSS 2.2 § | partial | src/layout/table.rs:1092-1109 divides spanning-cell min/preferred widt |
| border-collapse: separate paints independent cell borders | CSS 2.2 §17.6.1; CSS Tables 3  | supported-tested | manifest tables-border-separate and tables-border-spacing-zero; src/re |
| border-collapse: collapse merges adjacent borders into share | CSS 2.2 §17.6.2; CSS Tables 3  | supported-tested | manifest tables-border-collapse covers equal adjacent collapsed border |
| Collapsed border conflict resolution: hidden wins, none lose | CSS 2.2 §17.6.2.1 | partial | src/render/pdf.rs:2779-2879 paints each cell edge independently with n |
| Collapsed table outer border sizing and half-border placemen | CSS 2.2 §17.6.2 | partial | src/layout/table.rs:506-578 derives outer collapsed left/right from fi |
| border-spacing accepts one or two nonnegative lengths in sep | CSS 2.2 §17.6.1; CSS Tables 3  | supported-tested | manifest tables-border-spacing, tables-border-spacing-two-value, table |
| border-spacing is ignored when border-collapse is collapse | CSS Tables 3 §3.5.2 and §3.5.2 | supported-untested | src/render/pdf/layout_elements.rs:282-299 and src/layout/table.rs:841- |
| empty-cells: show paints borders/backgrounds for empty cells | CSS 2.2 §17.6.1.1 | supported-tested | default show behavior is exercised by ordinary separated-table fixture |
| empty-cells: hide suppresses empty-cell borders/backgrounds, | CSS 2.2 §17.6.1.1 | supported-tested | manifest tables-empty-cells-hide and tables-empty-cells-whitespace; sr |
| empty-cells: hide treats visibility:hidden content as no vis | CSS 2.2 §17.6.1.1 | partial | src/layout/table.rs:96-108 checks only DOM text/element presence, not  |
| vertical-align: top, middle, bottom align table cell content | CSS 2.2 §17.5.3 | supported-tested | manifest tables-cell-vertical-align; src/render/pdf/layout_elements.rs |
| vertical-align: baseline aligns baselines across cells and c | CSS 2.2 §17.5.3 | supported-tested | manifest tables-vertical-align-baseline and tables-vertical-align-base |
| Non-cell vertical-align values on table cells (sub, super, t | CSS 2.2 §17.5.3 | partial | VerticalAlign enum omits length/percentage and src/render/pdf/layout_e |
| caption-side: top and bottom position table captions before/ | CSS Tables 3 §3.5.3; CSS 2.2 § | supported-tested | manifest tables-caption, tables-caption-side-bottom, multipage-caption |
| Multiple table-caption boxes around one table are all laid o | CSS Tables 3 table-caption box | partial | src/layout/table.rs:618-632 stores only the first caption in an Option |
| display: table-caption creates captions from non-caption ele | MDN display table/internal val | unsupported | internal display values are not parsed or laid out; only HtmlTag::Capt |
| Default th presentation: bold, centered header cells with ta | HTML UA stylesheet behavior pl | supported-tested | manifest tables-th-header and tables-th-default; defaults in src/style |
| Row, row-group, column, column-group, table, and cell backgr | CSS 2.2 §17.5.1 and §17.6.1; C | partial | manifest covers row-group backgrounds only; source falls back from cel |
| visibility: collapse for table rows, columns, row groups, an | CSS 2.2 §17.5.5; CSS Tables 3  | partial | src/layout/table.rs:1229-1329 has a row-only collapsed-border special  |

### text-inline-fonts-generated
_The existing parity manifests cover the common horizontal text path well: line-height number/length, basic vertical-align keywords, normal/pre/nowrap/pre-wrap/pre-line white-space, text-align right/center/justify, letter/word spacing, text-indent length, simple case transforms, basic font families/sizes/weight/style, common list marker types and positions, ordered-list continuation across pages, basic counters, ::before/::after strings/attrs/urls/counters/quotes, ::first-line color/background, and ::first-letter color/transform/drop-cap. The blind spots are mostly value-space edges and cross-feature flows: percentage line metrics, length/percentage vertical-align, direction-sensitive logical alignment, source-only tab/writing-mode/font-variant/list-image support, richer OpenType controls, CJK/bidi/writing-mode details, marker content, counter-set/reversed counters, and geometry-affecting ::first-line styling._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| text-transform: none / uppercase / lowercase / capitalize | CSS Text 3 #propdef-text-trans | supported-tested | inline-text-text-transform-uppercase/lowercase/capitalize and fonts-ad |
| text-transform: full-width / full-size-kana | CSS Text 3 #propdef-text-trans | na-not-pdf-relevant | Fetched CSS Text 3 marks both values at-risk in /tmp/css-text-3.first2 |
| white-space: normal / nowrap / pre / pre-wrap / pre-line | CSS Text 3 #propdef-white-spac | supported-tested | inline-text and text-advanced manifest ids cover normal, nowrap, pre,  |
| white-space: break-spaces | CSS Text 3 #propdef-white-spac | partial | inline-text-white-space-break-spaces implemented but text-advanced-whi |
| white-space collapsing, segment breaks, trimming, preserved  | CSS Text 3 #white-space-proces | supported-tested | inline-text-white-space-normal/pre/pre-line and text-advanced-white-sp |
| tab-size: <number> | CSS Text 3 #propdef-tab-size | supported-untested | text-advanced-tab-size is expected_support unsupported, but src/style/ |
| tab-size: <length> | CSS Text 3 #propdef-tab-size | na-not-pdf-relevant | Fetched CSS Text 3 marks <length> tab-size at-risk in /tmp/css-text-3. |
| overflow-wrap: normal / break-word / anywhere and word-wrap  | CSS Text 3 #overflow-wrap-prop | supported-tested | inline-text-overflow-wrap-break-word and text-advanced-overflow-wrap-a |
| word-break: break-all | CSS Text 3 #word-break-propert | partial | inline-text-word-break-break-all implemented but text-advanced-word-br |
| word-break: keep-all | CSS Text 3 #word-break-propert | partial | text-advanced-word-break-keep-all only covers spaced Latin text; no so |
| line-break: auto / loose / normal / strict / anywhere | CSS Text 3 #line-break-propert | unsupported | No manifest entry; rg found no line-break handling under src/ |
| hyphens: none / manual / auto | CSS Text 3 #hyphens-property | unsupported | text-advanced-hyphens-auto expected_support unsupported; rg finds no h |
| soft wrap opportunities after hyphen-minus inside words | CSS Text 3 #line-breaking | supported-untested | No parity manifest id; src/layout/text.rs:429-436 documents and implem |
| text-align: left / right / center / justify | CSS Text 3 #text-align-propert | supported-tested | inline-text-text-align-right/center/justify and justify-multiline; src |
| text-align: start / end / match-parent | CSS Text 3 #text-align-propert | unsupported | No manifest entry; src/style/computed.rs:3137-3144 only recognizes cen |
| text-align-last | CSS Text 3 #propdef-text-align | unsupported | No manifest entry; rg found no text-align-last handling under src/ |
| text-justify | CSS Text 3 #propdef-text-justi | na-not-pdf-relevant | Fetched CSS Text 3 marks text-justify at-risk in /tmp/css-text-3.first |
| letter-spacing: normal / <length>, including negative length | CSS Text 3 #letter-spacing-pro | supported-tested | inline-text-letter-spacing and inline-text-letter-spacing-negative; sr |
| word-spacing: normal / <length>, including negative length | CSS Text 3 #word-spacing-prope | supported-tested | inline-text-word-spacing and inline-text-word-spacing-negative; src/st |
| word-spacing: <percentage> | CSS Text 3 #word-spacing-prope | unsupported | No manifest entry; src/style/computed.rs:4487-4489 only consumes CssVa |
| text-indent: <length> | CSS Text 3 #text-indent-proper | supported-tested | inline-text-text-indent-length and text-advanced-text-indent; src/layo |
| text-indent: <percentage> | CSS Text 3 #text-indent-proper | unsupported | No manifest entry; src/style/computed.rs:4450-4452 only consumes CssVa |
| text-indent: hanging / each-line | CSS Text 3 #text-indent-proper | na-not-pdf-relevant | Not included in stable gap set because interoperable browser support i |
| text-overflow: clip / ellipsis | CSS Overflow 3 #propdef-text-o | partial | text-advanced-text-overflow-ellipsis/clip expected_support partial; sr |
| text-overflow: <string> | CSS Overflow 3 #propdef-text-o | unsupported | text-advanced-text-overflow-string expected_support unsupported; src/s |
| line-height: normal / <number> / <length> | CSS Inline 3 #propdef-line-hei | supported-tested | typography-line-height-numeric/length and inline-text-line-height-*; s |
| line-height: <percentage> | CSS Inline 3 #propdef-line-hei | unsupported | No manifest entry; parser can produce Percentage but src/style/compute |
| mixed inline font sizes contributing to line-box height and  | CSS Inline 3 #inline-height | supported-untested | No direct parity id for mixed font-size leading; src/layout/text.rs:52 |
| vertical-align keyword baseline / sub / super / top / middle | CSS Inline 3 #propdef-vertical | supported-tested | inline-text vertical-align fixtures and typography sub/sup; src/style/ |
| vertical-align: text-bottom | CSS Inline 3 #propdef-vertical | supported-untested | No manifest id for text-bottom; src/style/computed.rs:4501-4505 and sr |
| vertical-align: <length> / <percentage> | CSS Inline 3 #propdef-vertical | unsupported | No manifest entry; src/style/computed.rs:4492-4505 only consumes CssVa |
| inline-block baseline, last-line baseline, shrink-to-fit inl | CSS Inline 3 / CSS2 inline for | supported-tested | inline-text-inline-block-baseline, baseline-multiline, shrink-to-fit,  |
| alignment-baseline, baseline-source, baseline-shift longhand | CSS Inline 3 #baseline-shift-p | na-not-pdf-relevant | CSS Inline 3 is WD and these longhands are not broadly interoperable f |
| initial-letter property | CSS Inline 3 #propdef-initial- | unsupported | No manifest entry; rg found no initial-letter property handling under  |
| font-family generic serif / sans-serif / monospace and fallb | CSS Fonts 4 #font-family-prop | supported-tested | typography-font-family-serif/sans-serif/monospace; src/style/computed. |
| @font-face font-family + src:url() local/relative font regis | CSS Fonts 4 #font-face-rule | partial | fonts-advanced-font-face-custom-src expected_support partial; src/pars |
| @font-face descriptors beyond family/src: font-weight, font- | CSS Fonts 4 #font-face-rule | unsupported | No target manifest coverage; src/parser/css/page.rs:50-83 ignores desc |
| font-size: px / pt / em / rem / percentage | CSS Fonts 4 #font-size-prop | supported-tested | typography-font-size-px/pt/em/rem/percent and fonts-advanced font-size |
| font-size: ex / ch | CSS Fonts 4 / CSS Values font- | supported-untested | fonts-advanced-font-size-ex/ch expected_support unsupported, but src/s |
| font-weight: normal / bold | CSS Fonts 4 #font-weight-prop | supported-tested | typography-font-weight-bold/normal; src/style/computed.rs:2970-2976 ma |
| font-weight numeric 1-1000 and relative bolder/lighter | CSS Fonts 4 #font-weight-prop | partial | No parity fixture for numeric ladder; src/style/computed.rs:2970-2976  |
| font-style: normal / italic / oblique | CSS Fonts 4 #font-style-prop | partial | typography-font-style-italic tests italic; src/style/computed.rs:2978- |
| font-stretch / font-width | CSS Fonts 4 #font-stretch-prop | unsupported | fonts-advanced-font-stretch-condensed expected_support unsupported; rg |
| font-variant-caps / font-variant: small-caps | CSS Fonts 4 #font-variant-caps | supported-untested | fonts-advanced-font-variant-small-caps expected_support unsupported, b |
| font-feature-settings OpenType feature tags, especially liga | CSS Fonts 4 #font-feature-sett | partial | fonts-advanced-font-feature-settings-ligatures expected_support unsupp |
| font-variant-ligatures | CSS Fonts 4 #font-variant-liga | unsupported | No manifest entry; rg found no font-variant-ligatures handling under s |
| font-variant-numeric | CSS Fonts 4 #font-variant-nume | unsupported | No manifest entry; rg found no font-variant-numeric handling under src |
| font-variant-east-asian | CSS Fonts 4 #font-variant-east | unsupported | No manifest entry; rg found no font-variant-east-asian handling under  |
| font-kerning | CSS Fonts 4 #font-kerning-prop | unsupported | No manifest entry; rg found no font-kerning property handling under sr |
| font-variation-settings and variable font axes | CSS Fonts 4 #font-variation-se | unsupported | No manifest entry; rg found no font-variation-settings handling under  |
| font-optical-sizing | CSS Fonts 4 #font-optical-sizi | unsupported | No manifest entry; rg found no font-optical-sizing handling under src/ |
| font-size-adjust | CSS Fonts 4 #font-size-adjust- | unsupported | No target manifest entry; rg only finds unrelated SVG parser tests, no |
| font-synthesis property controlling faux bold/italic/small-c | CSS Fonts 4 #font-synthesis-pr | unsupported | Source has internal faux oblique/bold comments, but rg found no font-s |
| font shorthand | CSS Fonts 4 #font-prop | unsupported | No target manifest entry; rg found no font shorthand decoder for HTML/ |
| direction: ltr / rtl and dir attribute inheritance | CSS Writing Modes 4 #direction | partial | text-advanced-direction-ltr implemented and direction-rtl expected_sup |
| unicode-bidi: normal / bidi-override / isolate-override | CSS Writing Modes 4 #unicode-b | partial | text-advanced-unicode-bidi-override expected_support unsupported; src/ |
| mixed-script Unicode bidi reordering without explicit unicod | CSS Writing Modes 4 #text-dire | supported-untested | No target parity id for mixed-script visual order; src/layout/text.rs: |
| writing-mode: horizontal-tb | CSS Writing Modes 4 #block-flo | supported-tested | Default behavior exercised by all horizontal inline-text fixtures; src |
| writing-mode: vertical-rl | CSS Writing Modes 4 #block-flo | supported-untested | text-advanced-writing-mode-vertical-rl expected_support unsupported, b |
| writing-mode: vertical-lr / sideways-rl / sideways-lr | CSS Writing Modes 4 #block-flo | unsupported | No manifest entry; src/style/computed.rs:4400-4407 explicitly falls un |
| text-orientation: mixed / upright / sideways | CSS Writing Modes 4 #text-orie | unsupported | No manifest entry; rg found no text-orientation parser, and src/render |
| text-combine-upright | CSS Writing Modes 4 #text-comb | unsupported | No manifest entry; rg found no text-combine-upright handling under src |
| list-style-type: disc / circle / square / decimal / decimal- | CSS Lists 3 #text-markers | supported-tested | lists-counters manifest covers all listed built-in types; src/style/co |
| list-style-type predefined counter styles beyond latin/roman | CSS Lists 3 / CSS Counter Styl | unsupported | No target manifest entry; src/style/computed.rs:5124-5138 unknown list |
| list-style-position: inside / outside | CSS Lists 3 #list-style-positi | supported-tested | list-style-position-inside/outside; src/layout/engine.rs:2769-3031 han |
| list-style-image: url() | CSS Lists 3 #image-markers | supported-untested | list-style-image-data-uri expected_support unsupported, but src/style/ |
| list-style shorthand | CSS Lists 3 #list-style-proper | supported-tested | list-style-shorthand; src/style/computed.rs:5093-5107 decodes type, po |
| ::marker color and font styling | CSS Pseudo 4 #marker-pseudo | supported-tested | marker-pseudo-color; src/layout/engine.rs:2817-2875 resolves marker ps |
| ::marker { content: ... } overriding default marker | CSS Pseudo 4 #marker-pseudo | supported-untested | No manifest id for marker content; src/layout/engine.rs:2827-2846 reso |
| counter-reset and counter-increment | CSS Lists 3 #auto-numbering | supported-tested | counter-reset-increment and generated-content-counter; src/layout/engi |
| counter-set | CSS Lists 3 #propdef-counter-s | unsupported | No manifest entry; rg found no counter-set handling under src/style |
| counter() with explicit counter style | CSS Lists 3 #counter-functions | supported-tested | generated-content-counter-roman implemented; lists-counters counter-co |
| counters() nested counter chains and scope | CSS Lists 3 #counter-functions | supported-tested | generated-content-counters-nested implemented; lists-counters counters |
| reversed counters and reversed() counter-reset | CSS Lists 3 #reversed-counters | unsupported | No manifest entry; rg found no reversed counter handling under src/ |
| HTML ol start and nested ol restart behavior | CSS Lists 3 #ua-stylesheet | supported-tested | ol-start-attribute and ol-decimal-markers |
| HTML ol reversed and li value attributes | CSS Lists 3 / HTML list number | unsupported | No target manifest entry; rg found ol start handling but no ol reverse |
| content: normal / none on ::before/::after | CSS Content 3 #content-propert | supported-tested | generated-content-content-none and generated-content-content-normal; s |
| content string concatenation and attr() | CSS Content 3 #content-propert | supported-tested | generated-content-before-string/after-string/attr/attr-missing/content |
| content: url() replaced pseudo-element image | CSS Content 3 #content-propert | supported-tested | generated-content-content-url-image and content-url-sized; src/layout/ |
| content: counter() and counters() | CSS Content 3 #content-propert | supported-tested | generated-content-counter, generated-content-counters-nested, generate |
| content open-quote / close-quote / no-open-quote / no-close- | CSS Content 3 #quotes-property | supported-tested | generated-content-open-close-quote, no-quote-keywords, nested-quotes;  |
| content target-counter(), target-counters(), target-text(),  | CSS Content 3 | na-not-pdf-relevant | CSS Content 3 is WD and these generated-content functions are not broa |
| ::before and ::after generated boxes: inline, block, inline- | CSS Pseudo 4 #generated-conten | supported-tested | generated-content-before-string/after-string/before-block/before-decor |
| ::before/::after suppression on replaced elements | CSS Pseudo 4 #generated-conten | supported-tested | generated-content-after-replaced |
| ::first-line color/font-weight/background/text-decoration su | CSS Pseudo 4 #first-line-pseud | supported-tested | generated-content-first-line and first-line-background; src/layout/hel |
| ::first-line font-size and geometry-affecting line metrics | CSS Pseudo 4 #first-line-pseud | partial | No parity fixture for first-line font-size; src/layout/helpers.rs:826- |
| ::first-letter color/transform and floated drop-cap | CSS Pseudo 4 #first-letter-pse | supported-tested | generated-content-first-letter-color/transform/dropcap; src/layout/hel |
| ::first-letter punctuation-including first-letter unit | CSS Pseudo 4 #application-in-c | supported-untested | No manifest id for leading quote/punctuation; src/layout/helpers.rs:85 |
| ::selection and highlight pseudo-elements | CSS Pseudo 4 #highlight-pseudo | na-not-pdf-relevant | Interactive user selection/highlight state is excluded for static page |

### transforms-pos-overflow-images-units-box
_Current parity coverage is strong for ordinary 2D transform shorthand functions, transform-origin, basic absolute/relative/fixed positioning on a single page, z-index/source-order stacking, hidden/visible/scroll clipping, padding-box and border-radius clipping, basic raster/SVG image sizing, object-fit scale-down and common object-position forms, box-sizing, CSS 2.x margin collapse cases, percentage/absolute/calc/var units, flex/grid interactions, and common Selectors 4/cascade paths. The blind spots are mainly stale expected-unsupported fixtures that no longer lock positive behavior, print-specific fixed-position pagination, 3D transform semantics, Display 3 box-suppression/BFC values, far-edge object-position math, advanced selector/cascade features, and font/viewport/intrinsic sizing combinations._

| feature | spec | status | evidence |
|---------|------|--------|----------|
| transform property with 2D transform-list composition order | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-translate, transform |
| translate(), translateX(), translateY(), including percentag | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-translate, transform |
| scale(), scaleX(), scaleY(), omitted second argument and neg | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-scale, transforms-sc |
| rotate() angle units deg/rad/turn and negative angles | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-rotate, transforms-r |
| skewX() and skewY() | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-skew-x, transforms-s |
| two-axis skew() shorthand | CSS Transforms 1 | supported-untested | tests/parity/manifest/transforms.json: transforms-skew is marked expec |
| matrix(a,b,c,d,e,f) | CSS Transforms 1 | supported-untested | tests/parity/manifest/transforms.json: transforms-matrix is marked exp |
| transform-origin keywords, percentages, lengths, and default | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-origin-center, trans |
| transform-box reference box selection | CSS Transforms 1 | unsupported | No transform-box handling found under src/; TransformOrigin resolves a |
| transformed element establishes containing block for absolut | CSS Transforms 1 | supported-tested | tests/parity/manifest/positioning.json: positioning-transformed-contai |
| individual transform properties translate, rotate, scale and | CSS Transforms 2 | unsupported | No computed-style handling for CSS properties named translate/rotate/s |
| 3D transform functions rotateX/rotateY/rotate3d/translateZ/t | CSS Transforms 2 | partial | src/style/computed.rs:6132-6159 approximates rotateX/rotateY as 2D sca |
| perspective, perspective-origin, transform-style: preserve-3 | CSS Transforms 2 | unsupported | rg found no perspective/backface/transform-style parser or computed fi |
| position: static and relative offsets | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-static-flow, posit |
| position:absolute top/left/right/bottom insets and stretch w | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-absolute-top-left, |
| absolute containing block from positioned ancestor | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-absolute-containin |
| position:fixed in paged media, repeated on each printed page | CSS Position 3 | partial | tests/parity/manifest/positioning.json and interactions.json cover onl |
| position:sticky scroll sticking behavior | CSS Position 3 | na-not-pdf-relevant | Sticky's dynamic scroll response has no live scroll state in static PD |
| z-index integer/auto ordering for positioned boxes | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-zindex-higher, pos |
| stacking context interactions with transform and z-index | CSS Position 3 / CSS Transform | supported-tested | tests/parity/manifest/interactions.json: positioning-zindex-x-transfor |
| absolute/fixed boxes inside flex and grid containers | CSS Position 3 / CSS Display 3 | supported-tested | tests/parity/manifest/interactions.json: positioning-absolute-x-flex,  |
| overflow: visible and hidden | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-clip, ov |
| overflow: clip | CSS Overflow 3 | supported-untested | tests/parity/manifest/overflow-clipping.json: overflow-clip is marked  |
| overflow: scroll and auto as static print clipping | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-scroll-print-cl |
| overflow-x/overflow-y separate axes and visible/clip compute | CSS Overflow 3 | supported-untested | tests/parity/manifest/overflow-clipping.json: overflow-x-y-separate is |
| overflow clipping to padding box | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-padding-box-cli |
| overflow clipping combined with border-radius | CSS Overflow 3 / CSS Backgroun | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-border-r |
| overflow clipping on flex/grid items and nested clipping cha | CSS Overflow 3 / CSS Display 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-flex-ite |
| overflow-clip-margin expanding the clip edge | CSS Overflow 3 | unsupported | No overflow-clip-margin handling found under src/ |
| scrollbar-gutter layout reservation for scroll containers | CSS Overflow 3 | unsupported | No scrollbar-gutter handling found under src/ |
| text-overflow: clip and ellipsis on clipped nowrap text | CSS Overflow 3 | supported-tested | tests/parity/manifest/text-advanced.json: text-advanced-text-overflow- |
| PNG/JPEG replaced image loading and natural dimensions | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-basic-png, img-basic-j |
| CSS width/height and HTML width/height on replaced images | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-width-height, img-heig |
| replaced image max-width/max-height clamping | CSS Images 3 / CSS Sizing 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-percent-width, img-max |
| object-fit: fill/contain/cover/none | CSS Images 3 | supported-untested | tests/parity/manifest/images-replaced.json: img-object-fit-contain/cov |
| object-fit: scale-down | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-fit-scale-down- |
| object-position keywords, percentages, and near-edge lengths | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-position-percen |
| object-position far-edge length offsets such as right 20px b | CSS Images 3 | partial | src/style/computed.rs:5368-5398 comments that far-edge length offsets  |
| replaced content clipped to its content box after object-fit | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-fit-cover-posit |
| SVG image intrinsic sizing, viewBox, and preserveAspectRatio | CSS Images 3 / SVG 2 | supported-tested | tests/parity/manifest/images-replaced.json: img-svg-as-img-inner-rect  |
| inline SVG shapes, gradients, clip paths, and text as replac | CSS Images 3 / SVG 2 | partial | tests/parity/manifest/images-replaced.json includes partial SVG inline |
| image fragmentation and slicing across pages | CSS Fragmentation / CSS Images | supported-tested | tests/parity/manifest/images-replaced.json: img-monolithic-page-push,  |
| width/height/min-width/max-width/min-height/max-height lengt | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-width-explicit, bloc |
| aspect-ratio property deriving an auto size | CSS Sizing 4 (shipped) / CSS S | supported-untested | tests/parity/manifest/images-replaced.json: img-aspect-ratio-box is ma |
| box-sizing: content-box and border-box | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-box-sizing-border-bo |
| intrinsic sizing width:min-content and width:max-content | CSS Sizing 3 | supported-untested | No target parity fixture for min-content/max-content; src/style/comput |
| width:fit-content keyword | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-width-fit-content; s |
| fit-content(<length-percentage>) function | CSS Sizing 3 | partial | src/style/computed.rs:7014-7016 approximates grid fit-content() tracks |
| CSS 2.2 block box margins, padding, borders, explicit width/ | CSS 2.2 box model | supported-tested | tests/parity/manifest/block-box-model.json: block-basic, block-padding |
| vertical margin collapsing including sibling, parent/child,  | CSS 2.2 box model | supported-tested | tests/parity/manifest/block-box-model.json: block-margin-collapse-adja |
| percentage padding and margins resolving against containing  | CSS 2.2 box model / CSS Values | supported-tested | tests/parity/manifest/units-values.json: units-percent-padding, units- |
| absolute length units px, pt, pc, in, cm, mm, Q | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-px-pt, units-pc-mm-q; s |
| font-relative em and rem units | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-em, units-rem, units-em |
| font-relative ex and ch units in layout lengths | CSS Values 4 | partial | src/parser/css/values.rs:64-73 preserves Ex/Ch, but src/style/resolve. |
| viewport units vw, vh, vmin, vmax resolved against the print | CSS Values 4 | supported-untested | tests/parity/manifest/units-values.json: units-viewport-vmin-vmax is m |
| small/large/dynamic viewport units svw/svh/lvw/lvh/dvw/dvh a | CSS Values 4 | na-not-pdf-relevant | These distinguish dynamic visual/layout viewport states; static PDF ou |
| calc() arithmetic with mixed units and operator precedence | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-calc-mixed, units-calc- |
| min(), max(), and clamp() sizing functions | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-min-max, units-clamp; s |
| custom properties var() fallback and nested fallback in leng | CSS Values 4 / CSS Cascade 5 | supported-tested | tests/parity/manifest/units-values.json: units-var-basic, units-var-fa |
| scientific notation numeric values | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-scientific-numbers; src |
| display:block, inline, inline-block, none | CSS Display 3 | supported-tested | tests/parity/manifest/block-box-model.json and interactions.json cover |
| display:flex and display:grid as stable layout modes | CSS Display 3 | supported-tested | tests/parity/manifest/interactions.json: flex/grid nesting, flex-wrap- |
| display:contents box suppression while preserving children | CSS Display 3 | unsupported | src/style/computed.rs:25-34 Display enum lacks Contents; parser at src |
| display:flow-root establishing a new block formatting contex | CSS Display 3 | unsupported | src/style/computed.rs:25-34 Display enum lacks FlowRoot; parser at src |
| CSS Display 3 multi-keyword display syntax such as inline fl | CSS Display 3 | unsupported | src/style/computed.rs:3170-3179 accepts only single keywords none/inli |
| type, class, id, universal, attribute selectors and combinat | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: specificity, class/type/ |
| :nth-child(), :nth-last-child(), :nth-of-type(), first/last/ | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: nth-child odd/even/formu |
| :nth-child(An+B of <selector-list>) | Selectors 4 | unsupported | No nth-child 'of selector' parsing found; src/parser/css/selectors.rs: |
| :not(), :is(), :where() selector-list matching and specifici | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-not-ps |
| :has() following sibling relative selectors | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-has-ad |
| :has() child and descendant relative selectors | Selectors 4 | partial | src/parser/css/selectors.rs:707-746 states child/descendant relations  |
| @media print and print media query gating | CSS Conditional 3 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-media- |
| @supports property feature queries with and/or/not | CSS Conditional 3 | supported-untested | tests/parity/manifest/selectors-cascade.json: @supports simple case wa |
| @supports selector() feature queries | CSS Conditional 3 / Selectors  | partial | src/parser/css/media.rs:518-519 treats selector(:has(a)) as a lenient  |
| cascade specificity, source order, inline styles, !important | CSS Cascade 5 | supported-tested | tests/parity/manifest/selectors-cascade.json: specificity, source-orde |
| cascade layers @layer and revert-layer | CSS Cascade 5 | unsupported | No @layer processing found; src/parser/css/values.rs:8 parses revert-l |
| dynamic pseudo-classes :hover, :focus, :active and pointer/i | Selectors 4 | na-not-pdf-relevant | Static PDF has no interactive state; src/parser/css/selectors.rs:648-6 |
