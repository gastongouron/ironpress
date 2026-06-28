# ironpress — CSS/HTML feature & parity coverage tracker

> Living master doc. Tracks every PDF-relevant CSS/HTML feature, ironpress support + test status, and every coverage gap + its verified verdict, across adversarial spec-audit rounds (finders fetch the live W3C/WHATWG/MDN specs; every gap verdict comes from an actual ironpress-vs-oracle render).

## Status

- **Features audited:** 620 across 8 areas — supported-tested 223, unsupported 184, partial 129, supported-untested 64, na-not-pdf-relevant 17, na-not-stable 1, supported-untested-in-area 1, na-not-stable-no-fixture 1
- **Round 1 (capped ~12/area):** 96 candidate gaps -> **verified:** LOCKED 12 (now regression tests), REAL-BUG 82 (tracked-unsupported, fix pending), DROPPED 2
- **Round 2 (uncapped/exhaustive):** 352 additional candidate gaps found so far (5 areas reported) — patch+verify pending
- **Parity gate:** new tracked-unsupported gaps lower the whole-corpus % honestly; each flips to PASS as its engine-fix lands. The score-regression gate measures only the common (pre-existing) fixture set, so coverage additions never read as regressions.

## Round-1 REAL BUGS (verified ironpress defects, tracked-unsupported until fixed)

Sorted by diff%. Each has a committed fixture + a spec-correct oracle ref.

| fixture id | category | diff% | oracle | defect |
|---|---|---|---|---|
| paged-break-precedence-before-wins | paged-media | 100.0 | weasyprint | ironpress uses 2 pages; ref needs 3 |
| flexbox-display-inline-flex | flexbox | 100.0 | chrome | inline-flex becomes 2 pages/blocky |
| flexbox-fragmentation-wrap-pages | flexbox | 100.0 | chrome | flex lines do not paginate |
| linear-gradient-color-hint | backgrounds-gradients | 100.0 | chrome | color hint dropped to fallback |
| repeating-linear-gradient-px-stops | backgrounds-gradients | 100.0 | chrome | px repeating stops dropped |
| color-oklch | color-opacity | 100.0 | chrome | oklch() dropped to fallback |
| mix-blend-mode-luminosity | effects | 100.0 | chrome | luminosity treated as normal |
| block-display-contents | block-box-model | 100.0 | chrome | wrapper box not suppressed; extra page |
| selectors-cascade-layers-order | selectors-cascade | 100.0 | chrome | cascade layers not applied |
| multiple-gradient-layers | backgrounds-gradients | 99.9 | chrome | only one gradient/geometry survives |
| background-blend-mode-overlay | effects | 99.8 | chrome | overlay background blend not applied |
| background-clip-text-gradient | backgrounds-borders | 98.3 | chrome | text clip renders as rectangle/text |
| grid-template-shorthand | grid | 98.2 | chrome | grid-template shorthand ignored |
| paged-flex-column-fragmentation | paged-media | 98.0 | chrome | flex pagination differs by page |
| linear-gradient-alpha-stops | backgrounds-gradients | 95.6 | chrome | alpha stops treated opaque |
| grid-invalid-template-areas | grid | 95.1 | chrome | invalid non-rectangular areas are rendered as if valid |
| grid-subgrid-columns | grid | 94.7 | chrome | subgrid columns unsupported |
| background-position-edge-offsets | backgrounds-borders | 93.1 | chrome | right/bottom offsets ignored |
| background-origin-border-box | backgrounds-borders | 92.6 | chrome | border-box origin wrong |
| background-repeat-space-round | backgrounds-borders | 89.3 | chrome | space/round tiling not applied |
| clip-path-inset-content-box | clip-mask | 89.1 | chrome | content-box geometry ignored |
| flexbox-direction-rtl-row | flexbox | 87.9 | chrome | row starts left instead of RTL right |
| tables-css-display-table | tables | 72.8 | chrome | CSS table display roles stack instead of table layout |
| multicol-break-before-column | multicol | 64.0 | chrome | forced column break ignored |
| tables-rowspan-zero | tables | 63.6 | chrome | rowspan=0 clamps instead of spanning row group |
| grid-placement-longhands | grid | 63.5 | chrome | placement longhands ignored |
| generated-content-first-line-font-size | generated-content | 58.1 | chrome | first-line font-size does not reflow geometry |
| grid-order-auto-placement | grid | 55.6 | chrome | grid auto-placement ignores order-modified order |
| filter-grayscale-group-descendants | filters | 54.5 | chrome | filter not applied to descendants/text group |
| grid-overlap-z-index | grid | 53.7 | chrome | overlapping static grid items stack incorrectly |
| filter-url-svg-gaussian-blur | filters | 53.0 | chrome | SVG feGaussianBlur URL filter ignored |
| tables-auto-colspan-distribution | tables | 51.2 | chrome | colspan auto sizing uses wrong column distribution |
| img-object-position-far-edge-length | images-replaced | 48.1 | chrome | far-edge object-position offsets ignored |
| grid-intrinsic-track-keywords | grid | 48.1 | chrome | min-content/max-content tracks collapse to same sizing |
| flexbox-aspect-ratio-grow-cross-size | flexbox | 45.8 | chrome | grown aspect-ratio items collapse |
| grid-repeat-auto-fit-collapse | grid | 41.8 | chrome | auto-fit does not collapse/stretch as Chrome does |
| background-size-auto-length | backgrounds-borders | 37.8 | chrome | mixed auto/length keeps intrinsic size |
| paged-side-margin-boxes | paged-media | 37.8 | weasyprint | side margin boxes missing |
| grid-fit-content-track | grid | 37.8 | chrome | fit-content behaves like auto |
| grid-auto-columns-implicit | grid | 37.6 | chrome | implicit columns collapse/ignore grid-auto-columns |
| border-image-gradient-slice | backgrounds-borders | 36.8 | chrome | gradient border-image disappears |
| block-flow-root-float-containment | block-box-model | 36.4 | chrome | flow-root does not contain float |
| text-advanced-hyphens-manual-soft-hyphen | text-advanced | 35.2 | chrome | soft hyphen break not honored |
| inline-text-text-indent-percent | inline-text | 33.9 | chrome | percent indent not resolved from containing block |
| grid-repeat-auto-fill-count | grid | 32.2 | chrome | auto-fill hard-wraps after too few tracks |
| tables-empty-cells-hidden-content | tables | 32.0 | chrome | hidden-content row still paints in ironpress |
| border-style-3d-bevels | backgrounds-borders | 30.5 | chrome | groove/ridge/inset/outset flatten to solid |
| flexbox-anonymous-text-items | flexbox | 30.0 | chrome | direct text flex items dropped |
| typography-line-height-percent | typography | 29.5 | chrome | 200% line-height not applied |
| selectors-has-nth-of-supports | selectors-cascade | 29.5 | chrome | `@supports selector`, `:has`, `nth-child(of S)` not applied |
| filter-drop-shadow-currentcolor | filters | 27.8 | chrome | omitted drop-shadow color defaults black |
| transforms-individual-transform-box-content | transforms | 27.2 | chrome | individual transform props/transform-box ignored |
| flexbox-flex-basis-max-content | flexbox | 26.9 | chrome | max-content basis ignored |
| flexbox-justify-safe-center-overflow | flexbox | 24.0 | chrome | safe center behaves unsafe/centered |
| opacity-text-glyph-group | color-opacity | 23.6 | chrome | opacity applied per glyph, not group |
| multicol-column-rule-empty-suppressed | multicol | 22.7 | chrome | rules painted beside empty columns |
| mask-composite-exclude-layers | clip-mask | 22.7 | chrome | mask layers/composite ignored |
| flexbox-abspos-static-position | flexbox | 22.2 | chrome | abspos child anchors top-left |
| multicol-column-gap-percentage | multicol | 22.0 | chrome | percentage gap not applied |
| paged-gcpm-string-element-last | paged-media | 19.0 | weasyprint | last running header absent |
| filter-chain-order-contrast-blur | filters | 17.5 | chrome | authored filter order not preserved |
| clip-path-polygon-evenodd | clip-mask | 17.4 | chrome | evenodd hole ignored |
| positioning-fixed-repeats-pages | positioning | 15.0 | chrome | fixed header missing on repeated page |
| overflow-clip-margin | overflow-clipping | 11.5 | chrome | clip margin bleed clipped away |
| flexbox-baseline-empty-synthesis | flexbox | 10.8 | chrome | empty item top-aligns |
| flexbox-min-width-zero-auto-min | flexbox | 10.2 | chrome | text overflows instead of clipping |
| inline-text-vertical-align-text-bottom | inline-text | 7.8 | chrome | text-bottom geometry differs from Chrome |
| tables-border-hidden-conflict | tables | 7.4 | chrome | hidden collapsed border should suppress red shared edge |
| text-advanced-text-align-end-rtl | text-advanced | 7.1 | chrome | RTL `end` aligns wrong edge |
| tables-border-style-conflict | tables | 7.0 | chrome | double border should beat solid at equal width |
| units-ch-layout-width | units-values | 6.9 | chrome | `ch` width metric differs |
| inline-text-vertical-align-length | inline-text | 6.7 | chrome | length baseline shift ignored |
| paged-named-page-margin-box | paged-media | 6.4 | weasyprint | second page repeats BASE header |
| tables-fixed-overflow-clip | tables | 5.9 | chrome | nowrap text spills past overflow:hidden cell |
| tables-border-width-conflict | tables | 5.7 | chrome | wider blue collapsed border should win |
| box-shadow-elliptical-radius | backgrounds-borders | 5.4 | chrome | shadow uses scalar rounded rect |
| paged-footnote-policy-inline | paged-media | 4.4 | weasyprint | inline footnotes stack as blocks |
| text-advanced-writing-mode-vertical-rl-columns | text-advanced | 4.3 | chrome | vertical-rl column placement differs |
| lists-counters-counter-set | lists-counters | 3.0 | chrome | labels render 1,1,2 instead of 1,7,8 |
| tables-vertical-align-text-bottom | tables | 2.3 | chrome | text-bottom is treated like bottom, not baseline |
| fonts-advanced-small-caps-synthesis | fonts-advanced | 1.8 | chrome | small-caps glyph geometry differs |
| overflow-axis-visible-hidden-coercion | overflow-clipping | 0.7 | chrome | coerced overflow axis scrollbar/clip parity differs |

## Round-1 LOCKED (ironpress correct — new regression tests)

| fixture id | category | diff% | note |
|---|---|---|---|
| paged-break-after-right-blank | paged-media | 0.00 | right-page break inserts blank page |
| paged-blank-selector-background | paged-media | 0.00 | `:blank` page paints yellow |
| paged-table-header-footer-repeat | paged-media | 1.12 | thead/tfoot repeat locked |
| flexbox-column-wrap-align-content | flexbox | 0.04 | column-wrap align-content locked |
| flexbox-cross-auto-margin-one-sided | flexbox | 0.03 | one-sided cross auto margin locked |
| tables-collapse-ignores-spacing | tables | 0.05 | collapsed borders correctly ignore border-spacing |
| tables-colgroup-span-fixed | tables | 0.00 | fixed-layout col span widths match Chrome |
| grid-auto-flow-dense | grid | 0.00 | dense backfill matches Chrome |
| opacity-percentage | color-opacity | 0.21 | opacity:50% works |
| text-advanced-tab-size-four | text-advanced | 0.38 | tab-size:4 locked |
| lists-counters-list-style-image-data-uri | lists-counters | 0.31 | data PNG markers locked |
| units-intrinsic-vmin-aspect-ratio | units-values | 0.00 | intrinsic/vmin/aspect-ratio locked |

## Round-1 DROPPED (no valid oracle / not discriminating)

| fixture id | reason |
|---|---|
| tables-missing-cells-anonymous | not discriminating; Chrome and ironpress both showed no visible anonymous-cell fill |
| transforms-rotate-y-backface-hidden | both print oracles paint the green backface |

## Round-2 exhaustive gap inventory (per area, patch+verify pending)

| area | new gaps |
|---|---|
| borders-bg-gradients | 40 |
| filters-effects-clip-color | 118 |
| flexbox | 47 |
| text-inline-fonts-generated | 85 |
| transforms-pos-overflow-images-units-box | 62 |
| **total** | **352** |

## Feature matrix by area

### borders-bg-gradients
_The target manifests already cover core solid/rgba backgrounds, basic linear/radial/conic gradients, percentage hard stops, repeating gradient families, common background-position/size/origin cases, solid/dashed/dotted/double/none borders, per-side colors, border-width keywords, many border-radius expansions including percentages/ellipses/clamping, outline offsets, and simple box-shadow. The blind spots are concentrated in unimplemented CSS Backgrounds & Borders values (border-image, 3D border styles, attachment, repeat space/round, four-value position, mixed auto sizing), in arbitrary multi-layer backgrounds and per-layer geometry, and in gradient stop semantics/alpha plus radius propagation into clips and shadows._

| feature | spec | status | evidence |
|---|---|---|---|
| background shorthand: color, image, position/size, repeat, o | CSS Backgrounds & Borders 3  | partial | src/parser/css/inline.rs parses background shorthand but tracks on |
| background-color including solid colors, rgba(), transparent | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: background-color-solid, background-c |
| background-image: none and url() raster/SVG images | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-gradients: background-position-keyword, backg |
| image-set() as a background image with resolution selection | CSS Images 4 image-set() | unsupported | MDN image-set consulted as shipped; rg found no image-set parsing  |
| multiple background image layers with CSS list matching | CSS Backgrounds & Borders 3  | partial | manifest backgrounds-gradients: multiple-backgrounds-layered expec |
| per-layer background-position/background-size/background-rep | CSS Backgrounds & Borders 3  | partial | src/parser/css/inline.rs records background-layer-slots but comput |
| background-position one- and two-value syntax with keywords, | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-gradients: background-position-keyword; src/s |
| background-position three-/four-value edge-offset syntax suc | CSS Backgrounds & Borders 3  | unsupported | src/style/computed.rs parse_background_position handles len==1 and |
| background-size cover, contain, explicit length/percentage p | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-gradients: background-size-cover, background- |
| background-size mixed auto/length or auto/percentage pairs | CSS Backgrounds & Borders 3  | partial | src/style/computed.rs parse_background_size accepts exact auto or  |
| background-repeat repeat, no-repeat, repeat-x, repeat-y | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-gradients image sizing/position fixtures exer |
| background-repeat round and space | CSS Backgrounds & Borders 3  | unsupported | src/style/computed.rs BackgroundRepeat has no Round/Space variants |
| background-origin border-box, padding-box, content-box | CSS Backgrounds & Borders 3  | supported-untested | manifest backgrounds-gradients covers background-origin-content-bo |
| background-clip border-box, padding-box, content-box | CSS Backgrounds & Borders 3  | partial | manifest backgrounds-gradients: background-clip-padding-box expect |
| background-clip:text with text-shaped background painting | CSS Backgrounds & Borders 4/ | unsupported | src/style/computed.rs parse_background_clip maps text to Border in |
| background-attachment scroll/fixed/local | CSS Backgrounds & Borders 3  | unsupported | rg background-attachment in src/ found no parser/computed/render s |
| root/body canvas background propagation | CSS Backgrounds & Borders 3  | supported-untested | src/lib.rs has root/body canvas background handling; no target man |
| border-width longhands/shorthand including thin/medium/thick | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-solid-width, border-width-key |
| border-color longhands/shorthand including per-side, transpa | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-per-side-colors, border-color |
| border-style none, hidden, solid, dashed, dotted, double | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-style-dashed, border-style-do |
| border-style groove, ridge, inset, outset 3D rendering | CSS Backgrounds & Borders 3  | partial | src/style/computed.rs parse_border_style_keyword does not preserve |
| per-side border shorthand/longhand interaction and mixed sid | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-per-side-colors plus style/wi |
| border-radius shorthand one/two/three/four value expansion a | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-radius-corner-longhands, bord |
| elliptical border-radius slash syntax and percentage radii | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-radius-percentage, border-rad |
| border-radius overlap reduction/clamping when radii sums exc | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-radius-clamped; src/render/pd |
| border-radius applied consistently to borders, background cl | CSS Backgrounds & Borders 3  | partial | src/render/pdf.rs normal background path uses per-corner/elliptica |
| border-image-source/slice/width/outset/repeat and border-ima | CSS Backgrounds & Borders 3  | unsupported | rg border-image in src/ found no parser/computed/render support; n |
| outline and outline-offset in static output | CSS Basic UI / target backgr | supported-tested | manifest backgrounds-borders: outline-solid, outline-offset-negati |
| box-shadow outer shadows with offsets, color, currentColor,  | CSS Backgrounds & Borders 3  | supported-tested | manifest backgrounds-borders: border-box-shadow-offset, border-x-b |
| box-shadow blur radius, spread distance, negative spread, an | CSS Backgrounds & Borders 3  | supported-tested | tests/parity/manifest/effects.json covers blur/spread/inset/negati |
| box-shadow with nonuniform, percentage, or elliptical border | CSS Backgrounds & Borders 3  | partial | effects manifest covers simple border-radius shadow only; src/rend |
| linear-gradient() directions: default, angles, to-side, to-c | CSS Images 3 section 3.4 lin | supported-tested | manifest backgrounds-borders: background-linear-gradient, backgrou |
| linear-gradient() multiple color stops and hard stops with p | CSS Images 3 sections 3.4.2- | supported-tested | manifest backgrounds-gradients: linear-gradient-multi-stop, linear |
| repeating-linear-gradient() with percentage stops | CSS Images 3 section 3.4 rep | supported-tested | manifest backgrounds-gradients: repeating-linear-gradient; src/sty |
| gradient color-stop positions expressed as lengths, includin | CSS Images 3 section 3.4.3 c | unsupported | src/style/computed.rs parse_gradient_stops accepts percent tokens  |
| gradient color interpolation hints/midpoints between stops | CSS Images 3 section 3.4.3 c | unsupported | src/style/computed.rs parse_gradient_stops expects each comma item |
| gradient alpha/transparent stop compositing over underlying  | CSS Images 3 gradient color  | partial | src/style/computed.rs parse_gradient_color ignores rgba alpha and  |
| radial-gradient() circle/ellipse shapes and default center | CSS Images 3 section 3.5 rad | supported-tested | manifest backgrounds-borders: background-radial-gradient, backgrou |
| radial-gradient() size keywords closest-side, farthest-side, | CSS Images 3 section 3.5.1 r | supported-untested | manifest backgrounds-gradients covers closest-side and farthest-si |
| radial-gradient() explicit radii and at-position syntax | CSS Images 3 sections 3.5.1- | supported-tested | manifest backgrounds-gradients: radial-gradient-sized-px, radial-g |
| repeating-radial-gradient() | CSS Images 3 section 3.5 rep | supported-tested | manifest backgrounds-gradients: repeating-radial-gradient; src/sty |
| conic-gradient() basic, from angle, at position, and hard st | CSS Images 4 conic gradients | supported-tested | manifest backgrounds-borders: background-conic-gradient; manifest  |
| repeating-conic-gradient() | CSS Images 4 conic gradients | supported-tested | manifest backgrounds-gradients: repeating-conic-gradient; src/rend |
| conic-gradient() color hints/midpoints and non-angle stop fi | CSS Images 4 conic gradients | unsupported | src/style/computed.rs parse_conic_stops accepts colors plus angula |
| CSS image() and element() functions as backgrounds | CSS Images 4 image functions | na-not-pdf-relevant | image() and element() remain Level 4 / not broadly stable in the c |
| background-image: image-set() CSS image function | CSS Images 4 / MDN image-set | unsupported | Round 1 tagged image-set unsupported but had no gap; source backgr |
| background-attachment fixed and local in paged output | CSS Backgrounds 3 background | unsupported | Round 1 tagged attachment unsupported without a gap; ComputedStyle |
| root/body canvas background propagation and suppression rule | CSS Backgrounds 3 special ba | partial | Round 1 marked root/body propagation supported-untested; source ap |
| background-clip: content-box and padding-box on colors/gradi | CSS Backgrounds 3 background | partial | Round 1 only proposed background-clip:text; source has Border/Padd |
| background-origin on gradient tiles | CSS Backgrounds 3 background | partial | Renderer routes origin into raster/SVG background contexts; gradie |
| comma-separated background-origin and background-clip lists  | CSS Backgrounds 3 layered ba | unsupported | Source keeps single background_origin/background_clip fields, whil |
| background-repeat two-keyword per-axis syntax | CSS Backgrounds 3 background | unsupported | parse_background_repeat_value recognizes repeat/no-repeat/repeat-x |
| background-position-x and background-position-y longhands | CSS Backgrounds 4 / MDN back | unsupported | No background-position-x/y handling appears in the style or render |
| background-image none layers participate in list matching | CSS Backgrounds 3 layered ba | partial | Source records layer slots including none for size/position/repeat |
| background shorthand visual-box semantics | CSS Backgrounds 3 background | supported-untested | Round 1 background shorthand was partial; source has parsing for o |
| background shorthand with multiple image layers and final ba | CSS Backgrounds 3 background | unsupported | parse_background_shorthand tracks a single found_image slot and do |
| background-blend-mode including layer-list matching | Compositing and Blending 1 b | partial | Source parses a single BlendMode enum, but comma-separated layer m |
| border-width, border-style, and border-color 2/3/4-value sho | CSS Backgrounds 3 border pro | partial | Computed style applies uniform border-width/style/color values; pe |
| background and shadow clipping with nonuniform or elliptical | CSS Backgrounds 3 rounded co | partial | Round 1 had one outer shadow ellipse gap; renderer clip helpers st |
| gradient color-interpolation-method syntax such as in oklab  | CSS Images 4 gradients and C | unsupported | parse_linear_gradient treats the first argument as a direction/ang |
| radial/conic gradient at-position edge-offset syntax | CSS Images 3/4 gradient posi | unsupported | parse_radial_center accepts basic keywords, percentages, and lengt |
| radial-gradient corner extent keywords with off-center cente | CSS Images 3 radial-gradient | supported-untested | Round 1 noted closest-corner/farthest-corner were unisolated; pars |
| radial/conic color hints and repeating radial length range s | CSS Images 3 color stop list | unsupported | Round 1 covered linear hints and repeating-linear px stops; parser |
| gradient stop positions using calc() | CSS Images 3 color stop list | unsupported | parse_gradient_stops only treats tokens ending in a literal percen |

### filters-effects-clip-color
_Existing parity coverage is strongest for sRGB color syntax, basic opacity numeric/clamp cases, display/visibility, simple box-shadow variants, several gradient mask-image forms, -webkit-mask-image aliasing, simple clip-path shapes, and basic image/filter cases. The main blind spots are group semantics and operation order: filters are not proved on text/descendant source graphics, filter chains can be order-insensitive, clip-path fixtures do not exercise geometry boxes or fill-rule, mask fixtures do not cover multiple layers/composite/positioning, blending covers only multiply/screen plus one background case, non-separable blend modes are absent, and CSS Color 4 wide-gamut/perceptual functions plus opacity percentages are untested or unsupported._

| feature | spec | status | evidence |
|---|---|---|---|
| filter property: none and ordered <filter-value-list> | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-chained; src/style/comp |
| filter creates an atomic filtered group of the element and d | Filter Effects 1 | partial | src/layout/images.rs apply_color_filters_to_box recolors only box  |
| filter painting can extend outside the border box without ch | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-blur-box and filter-on- |
| filter compositing order: filter before clipping, masking, a | Filter Effects 1 | supported-untested | src/render/pdf.rs wraps opacity/clip/mask around rendered boxes, b |
| filter function blur() on raster images | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-blur-img; src/layout/im |
| filter function blur() on CSS boxes | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-blur-box and filter-on- |
| filter function blur() on text glyphs | Filter Effects 1 | partial | No manifest text blur filter fixture; source filtering is box/imag |
| filter function brightness() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-brightness is expected_ |
| filter function contrast() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-contrast expected unsup |
| filter function grayscale() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-grayscale expected unsu |
| filter function sepia() | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-sepia expected implemen |
| filter function saturate() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-saturate expected unsup |
| filter function hue-rotate() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-hue-rotate expected uns |
| filter function invert() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-invert expected unsuppo |
| filter function opacity() | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-opacity-fn expected uns |
| filter function drop-shadow() basic image alpha shadow | Filter Effects 1 | supported-tested | tests/parity/manifest/filters.json: filter-drop-shadow expected im |
| filter function drop-shadow() default color from currentColo | Filter Effects 1 | unsupported | src/style/computed.rs parse_drop_shadow defaults missing color to  |
| filter function drop-shadow() as one item in an ordered filt | Filter Effects 1 | partial | src/style/computed.rs keeps a single drop_shadow field and src/lay |
| filter url(#id) referencing SVG filter: feColorMatrix subset | Filter Effects 1 | partial | tests/parity/manifest/filters.json: filter-url-svg expected unsupp |
| filter url(#id) referencing SVG filter primitives such as fe | Filter Effects 1 | unsupported | src/parser/svg.rs filter_element_color_ops has no general SVG filt |
| color-interpolation-filters property for SVG filter primitiv | Filter Effects 1 | unsupported | No source handling found for color-interpolation-filters; CSS filt |
| filter animation/interpolation rules | Filter Effects 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output; only |
| filter hit testing and pointer behavior | Filter Effects 1 | na-not-pdf-relevant | Hit-testing behavior has no static PDF raster output |
| box-shadow offset and color | CSS Backgrounds / Compositin | supported-tested | tests/parity/manifest/effects.json: box-shadow-offset and box-shad |
| box-shadow blur radius | CSS Backgrounds / visual eff | partial | tests/parity/manifest/effects.json: box-shadow-blur marked partial |
| box-shadow positive spread | CSS Backgrounds / visual eff | partial | tests/parity/manifest/effects.json: box-shadow-spread and box-shad |
| box-shadow negative spread | CSS Backgrounds / visual eff | supported-tested | tests/parity/manifest/effects.json: box-shadow-negative-spread exp |
| box-shadow inset shadows | CSS Backgrounds / visual eff | partial | tests/parity/manifest/effects.json: box-shadow-inset partial and b |
| box-shadow multiple shadows and paint order | CSS Backgrounds / visual eff | partial | tests/parity/manifest/effects.json: box-shadow-multiple partial; s |
| box-shadow currentColor | CSS Color 4 / CSS Background | supported-tested | tests/parity/manifest/effects.json: box-shadow-currentcolor; src/s |
| box-shadow with border-radius | CSS Backgrounds / visual eff | supported-tested | tests/parity/manifest/effects.json: box-shadow-border-radius |
| text-shadow offset shadows | CSS Text Decoration / visual | partial | tests/parity/manifest/effects.json: text-shadow-offset expected un |
| text-shadow blur | CSS Text Decoration / visual | partial | tests/parity/manifest/effects.json: text-shadow-blur expected unsu |
| text-shadow multiple shadows and currentColor | CSS Text Decoration / CSS Co | supported-untested | src/style/computed.rs parses text_shadow as a list and resolves cu |
| mix-blend-mode: normal | Compositing and Blending 1 | supported-tested | Default rendering and BlendMode::Normal in src/style/computed.rs |
| mix-blend-mode: multiply | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: mix-blend-mode-multiply expect |
| mix-blend-mode: screen | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: mix-blend-mode-screen expected |
| mix-blend-mode separable values overlay, darken, lighten, co | Compositing and Blending 1 | supported-untested | src/style/computed.rs BlendMode includes these values and src/rend |
| mix-blend-mode non-separable values hue, saturation, color,  | Compositing and Blending 1 | unsupported | Compositing 1 defines these blend modes, but src/style/computed.rs |
| mix-blend-mode establishes a blended stacking context for te | Compositing and Blending 1 | partial | src/render/pdf.rs wraps container branches in blend states; top-le |
| background-blend-mode: multiply | Compositing and Blending 1 | partial | tests/parity/manifest/effects.json: background-blend-mode-multiply |
| background-blend-mode other separable values | Compositing and Blending 1 | supported-untested | src/style/computed.rs BlendMode supports separable PDF blend names |
| background-blend-mode non-separable values hue, saturation,  | Compositing and Blending 1 | unsupported | src/style/computed.rs BlendMode lacks non-separable blend modes |
| background-blend-mode list matching multiple background laye | Compositing and Blending 1 | unsupported | ComputedStyle has a single background_blend_mode field; parser doe |
| isolation property: auto and isolate | Compositing and Blending 1 | unsupported | No source handling found for isolation; important for constraining |
| compositing animation/interpolation | Compositing and Blending 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output |
| clip-path: inset() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-inset expected uns |
| clip-path: inset() round corners | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-inset-round expect |
| clip-path: circle() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-circle expected un |
| clip-path: ellipse() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-ellipse expected u |
| clip-path: polygon() basic shape | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: clip-path-polygon expected u |
| clip-path basic shapes with reference geometry boxes border- | CSS Masking 1 | unsupported | src/style/computed.rs parse_clip_path does not parse shape-box or  |
| clip-path polygon() fill-rule nonzero/evenodd | CSS Masking 1 / CSS Shapes | unsupported | src/style/computed.rs parse_clip_path does not parse polygon fill- |
| clip-path url(#clipPath) SVG clip source | CSS Masking 1 | unsupported | No clip-path url handling found; parser handles only circle/ellips |
| clip-rule property for referenced SVG clipPath | CSS Masking 1 | unsupported | No source handling found for clip-rule; relevant when clip-path re |
| deprecated clip: rect(...) on absolutely positioned elements | CSS Masking 1 | unsupported | Spec requires support, but no parser/render handling found for the |
| mask-image: none | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-none |
| mask-image linear-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-linear-gradient a |
| mask-image radial-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-radial-gradient |
| mask-image conic-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-conic-gradient |
| mask-image repeating-linear-gradient() | CSS Masking 1 | supported-tested | tests/parity/manifest/clip-mask.json: mask-image-repeating-linear |
| mask-image repeating-radial-gradient() and repeating-conic-g | CSS Masking 1 | partial | src/render/pdf.rs rasterize_mask_coverage handles repeating linear |
| mask-image url() SVG/image masks | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: mask-image-url-svg expected  |
| -webkit-mask-image alias for compatibility | CSS Masking 1 / browser-ship | supported-tested | tests/parity/manifest/clip-mask.json: webkit-mask-image-alias |
| mask-mode: alpha, luminance, match-source | CSS Masking 1 | partial | tests/parity/manifest/clip-mask.json: mask-mode-luminance; src/sty |
| mask-position, mask-repeat, mask-size | CSS Masking 1 | unsupported | Parser may retain raw CssValue, but ComputedStyle/render mask path |
| mask-origin and mask-clip | CSS Masking 1 | unsupported | No render-time geometry mapping found for mask-origin or mask-clip |
| multiple mask layers | CSS Masking 1 | unsupported | src/style/computed.rs parse_mask_image keeps a single MaskSource a |
| mask-composite: add, subtract, intersect, exclude | CSS Masking 1 | unsupported | No ComputedStyle field or render path applying mask-composite betw |
| mask shorthand full grammar | CSS Masking 1 | partial | src/style/computed.rs maps mask through parse_mask_image only; pos |
| mask-type on SVG <mask>: luminance and alpha | CSS Masking 1 | partial | src/render/pdf.rs includes SVG alpha/luminance conversion logic, b |
| mask-border-* family | CSS Masking 1 | unsupported | No source handling found for mask-border-source/slice/width/outset |
| mask animation/interpolation | CSS Masking 1 | na-not-pdf-relevant | Animation timelines are excluded for static paged PDF output |
| CSS named colors including CSS Color 4 named set | CSS Color 4 | partial | tests/parity/manifest/color-opacity.json tests navy/rebeccapurple/ |
| transparent keyword | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-transparent |
| currentColor keyword in dependent colors | CSS Color 4 | partial | tests/parity/manifest/color-opacity.json: color-currentcolor-borde |
| hex colors #rgb and #rrggbb | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hex-short and colo |
| hex colors with alpha #rgba and #rrggbbaa | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hex-alpha-short an |
| legacy rgb()/rgba() comma syntax | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-rgb and color-rgba |
| modern rgb() space/slash syntax, percentages, none component | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-rgb-modern-slash,  |
| alpha values in color functions as number or percentage | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-alpha-percentage a |
| hsl()/hsla() legacy and modern syntax, hue angles, powerless | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hsl, color-hsla, c |
| hwb() | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-hwb and color-hwb- |
| lab() and lch() | CSS Color 4 | unsupported | No manifest fixture or source parser support found for lab()/lch() |
| oklab() and oklch() | CSS Color 4 | unsupported | No manifest fixture or source parser support found for oklab()/okl |
| color() function with srgb and srgb-linear | CSS Color 4 | unsupported | No manifest fixture or source parser support found for color(<pred |
| color() function with display-p3, a98-rgb, prophoto-rgb, rec | CSS Color 4 | unsupported | No manifest fixture or source parser support found for wide-gamut  |
| opacity property with numeric values and clamping | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: opacity-half, opacity-cl |
| opacity property with percentage values | CSS Color 4 | unsupported | CSS Color 4 allows percentages; src/style/computed.rs opacity assi |
| opacity group flattening for nested boxes | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: opacity-nested-group |
| opacity group flattening for overlapping text glyphs | CSS Color 4 | supported-untested | No manifest fixture forces overlap within one translucent text ele |
| visibility: hidden, collapse, and visible descendants | CSS Display / Color visibili | supported-tested | tests/parity/manifest/color-opacity.json: visibility-hidden, visib |
| display: none | CSS Display | supported-tested | tests/parity/manifest/color-opacity.json: display-none |
| text glyph color from color property | CSS Color 4 | supported-tested | tests/parity/manifest/color-opacity.json: color-text-glyph |
| system colors and forced-color dynamic adaptation | CSS Color 4 | na-not-pdf-relevant | Static PDF comparison should not depend on interactive forced-colo |
| backdrop-filter property on static transparent boxes | MDN backdrop-filter / Filter | unsupported | MDN documents backdrop-filter as applying graphical effects to the |
| filter invalid function invalidates the declaration instead  | Filter Effects 1 filter valu | unsupported | src/style/computed.rs parse_filter loops over functions and ignore |
| filter url(#missing-or-not-filter) makes the whole filter ch | Filter Effects 1 §filter pro | unsupported | parse_filter records url ids and still preserves other parsed func |
| SVG filter primitive feOffset in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | src/parser/svg.rs filter_element_color_ops only reads feColorMatri |
| SVG filter primitive feBlend in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | No feBlend renderer is present in the CSS filter:url() path. |
| SVG filter primitive feComposite in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | No feComposite renderer is present in the CSS filter:url() path. |
| SVG filter primitives feFlood and feMerge in CSS filter:url( | Filter Effects 1 SVG filter  | unsupported | filter_element_color_ops skips all non-feColorMatrix children. |
| SVG filter primitive feComponentTransfer in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | No feComponentTransfer parsing or transfer-function rendering was  |
| SVG filter primitive feMorphology in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | No feMorphology support was found in parser or renderer paths for  |
| SVG filter primitive feDropShadow in CSS filter:url() | Filter Effects 1 SVG filter  | unsupported | CSS drop-shadow() is parsed separately, but SVG feDropShadow insid |
| SVG filter primitives feTurbulence and feDisplacementMap in  | Filter Effects 1 SVG filter  | unsupported | No procedural SVG filter primitive implementation was found. |
| clip-path: path() basic shape | CSS Masking 1 clip-path refe | unsupported | src/style/computed.rs parse_clip_path handles circle/ellipse/inset |
| clip-path: xywh() and rect() basic shapes | CSS Masking 1 clip-path refe | unsupported | parse_clip_path has no xywh() or rect() arms. |
| clip-path basic-shape keyword radii and position keywords | CSS Masking 1 + CSS Shapes b | unsupported | parse_clip_path only parses numeric lengths/percentages; closest-s |
| clip-path geometry-box without a basic shape | CSS Masking 1 §5.1 | unsupported | parse_clip_path does not parse standalone border-box/content-box/m |
| mask-position longhand | CSS Masking 1 §7.4 | unsupported | ComputedStyle has mask_image/mask_mode but no mask-position field  |
| mask-size longhand | CSS Masking 1 §7.7 | unsupported | Mask soft-mask generation always maps the source to the element bo |
| mask-repeat longhand | CSS Masking 1 §7.3 | unsupported | Mask soft-mask generation treats gradients/SVG as one box-filling  |
| mask-origin and mask-clip geometry boxes | CSS Masking 1 §7.5-7.6 | unsupported | No render-time mask-origin or mask-clip geometry mapping was found |
| CSS Color 4 lab(), lch(), oklab(), predefined color(), and w | CSS Color 4 §§9-10 | unsupported | The local parser evidence found support for rgb/hsl/hwb via existi |
| CSS Color 5 color-mix() static color function | CSS Color 5 / MDN <color> | unsupported | No color-mix parsing was found; static final color is PDF-relevant |

### flexbox
_The existing flexbox manifest is strong for core row/column flex layout: display:flex, physical directions and reverse directions, nowrap/wrap/wrap-reverse, flex-flow, order including negative ties, justify-content and align-items basics, align-content for wrapped rows, grow/shrink/basis including fractional sums and clamps, fixed and percentage gaps, main-axis auto margins, nested flex containers, min/max clamps, and percent-height stretch. Blind spots remain around unsupported parse surface, generated anonymous items, logical axis mapping, safe overflow alignment, synthesized baselines, abspos static-position behavior, flexed aspect-ratio sizing, column-wrap cross distribution, auto-min escape hatches, one-sided cross auto margins, intrinsic flex-basis keywords, and paged fragmentation._

| feature | spec | status | evidence |
|---|---|---|---|
| display:flex creates a block-level flex container | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-display-flex; src/style/computed.rs:3170-3178  |
| display:inline-flex creates an inline-level flex container | https://www.w3.org/TR/css-fl | unsupported | no manifest entry; src/style/computed.rs:3170-3178 parses flex but |
| flex item generation for element children | https://www.w3.org/TR/css-fl | supported-tested | many manifest entries use element children; src/layout/flex.rs:170 |
| anonymous flex items from direct text children | https://www.w3.org/TR/css-fl | unsupported | no manifest entry; src/layout/flex.rs:170-179 filters only DomNode |
| absolutely positioned flex children are out of flex flow and | https://www.w3.org/TR/css-fl | partial | no flexbox manifest entry; src/layout/flex.rs:183-291 excludes abs |
| flex-direction row, row-reverse, column, column-reverse | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-flex-direction-column, flexbox-flex-direction |
| main-axis mapping through direction and writing-mode | https://www.w3.org/TR/css-fl | partial | no manifest entry; src/layout/flex.rs:1338-1380 treats row as phys |
| flex-wrap nowrap, wrap, wrap-reverse for rows | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-flex-wrap-nowrap, flexbox-flex-wrap, flexbox- |
| flex-direction:column with flex-wrap creates additional colu | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-column-wrap; src/layout/flex.rs:1380-1468 has  |
| flex-flow shorthand combines direction and wrap | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-flex-flow; src/style/computed.rs:3182-3191 par |
| order property reorders layout and paint order, including ne | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-order and flexbox-order-negative; src/layout/ |
| static flex item z-index creates flex painting order effect | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-z-index-static; src/render/pdf.rs keeps FlexCe |
| flex-grow positive free-space distribution | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-flex-grow, flexbox-flex-grow-basis-auto; src/ |
| flex-grow factors whose sum is less than one | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-grow-fractional; src/layout/flex.rs:2022-2033  |
| flex-grow max-main clamp and freeze redistribution | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-max-width-clamp; src/layout/flex.rs:2045-2067  |
| flex-shrink scaled shrink factors and negative free-space di | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-flex-shrink and flexbox-flex-shrink-zero; src |
| flex-shrink factors whose sum is less than one | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-shrink-fractional; src/layout/flex.rs:2081-208 |
| flex-shrink min-main clamp and freeze redistribution | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-min-width-constraint, flexbox-min-height-colu |
| flex-basis length, auto, content, zero, and percentage | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-flex-basis, flexbox-flex-basis-auto, flexbox- |
| flex-basis intrinsic sizing keywords min-content, max-conten | https://www.w3.org/TR/css-fl | unsupported | no manifest entry; src/style/computed.rs:3287-3321 handles auto/co |
| flex shorthand keywords and numeric forms | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-flex-shorthand-keywords; src/style/computed.rs |
| automatic minimum main size content floor for min-width:auto | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-min-content-no-overflow; src/layout/flex.rs:10 |
| automatic minimum size escape hatches: min-width:0 or non-vi | https://www.w3.org/TR/css-fl | supported-untested | no flexbox manifest entry; src/layout/flex.rs:1094-1118 only appli |
| min/max main-axis and cross-axis clamps | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-max-width-clamp, flexbox-min-height-column, f |
| justify-content flex-start, flex-end, center, space-between, | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-justify-content-*; src/layout/flex.rs:2328-23 |
| justify-content start/end/left/right aliases from CSS Box Al | https://www.w3.org/TR/css-al | partial | no manifest entry; src/style/computed.rs:3215-3219 parses aliases, |
| justify-content safe and unsafe overflow-position prefixes | https://www.w3.org/TR/css-al | partial | manifest id flexbox-justify-safe-center only covers fitting conten |
| align-items flex-start, flex-end, center, stretch, baseline | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-align-items-flex-start, flexbox-align-items-f |
| baseline synthesis for flex items without text baselines | https://www.w3.org/TR/css-fl | partial | no manifest entry; src/render/pdf.rs baseline path says no text ba |
| align-self auto, flex-start, flex-end, center, baseline, str | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-align-self-flex-end, flexbox-align-self-cente |
| align-content flex-start, flex-end, center, space-between, s | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-align-content-*; src/layout/flex.rs:1732-1807 |
| align-content distribution for column-wrap flex containers | https://www.w3.org/TR/css-fl | supported-untested | manifest id flexbox-column-wrap covers wrapping but not align-cont |
| main-axis auto margins absorb positive free space before jus | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-margin-auto-main-end, flexbox-margin-auto-spl |
| cross-axis auto margins override align-self and suppress str | https://www.w3.org/TR/css-fl | supported-untested | manifest id flexbox-margin-auto-center covers both-axis auto but n |
| fixed flex item margins do not collapse and participate in p | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-item-margin; src/layout/flex.rs:673-683 maps f |
| percentage margins and paddings on flex items resolve agains | https://www.w3.org/TR/css-fl | supported-tested | covered indirectly by units-values percent fixtures plus flex item |
| gap, row-gap, column-gap fixed lengths, two-value gap, and p | https://www.w3.org/TR/css-al | supported-tested | manifest ids flexbox-gap, flexbox-gap-two-value, flexbox-row-colum |
| nested flex containers as flex items | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-nested-flex, flexbox-nested-row-in-row, flexb |
| percentage-height descendants resolve after align-items:stre | https://www.w3.org/TR/css-fl | supported-tested | manifest id flexbox-percent-height-stretch; src/layout/flex.rs:155 |
| percentage flex-basis in a definite-height column flex conta | https://www.w3.org/TR/css-fl | supported-untested | no manifest entry; src/layout/flex.rs:1264-1296 resolves column pe |
| aspect-ratio flex items with flexible main sizes | https://www.w3.org/TR/css-fl | partial | no manifest entry; src/layout/flex.rs computes aspect-ratio height |
| subpixel and fractional free-space distribution/rounding | https://www.w3.org/TR/css-fl | supported-tested | manifest ids flexbox-grow-fractional and flexbox-shrink-fractional |
| fragmenting multi-line flex layout across pages | https://www.w3.org/TR/css-fl | unsupported | no manifest entry; src/layout/paginate.rs:125-139 and 1603-1614 es |
| two-keyword display syntax display:block flex and display:in | https://www.w3.org/TR/css-di | unsupported | CSS Display defines outer/inner display syntax and legacy inline-f |
| display:contents child of a flex container contributes its c | https://www.w3.org/TR/css-di | unsupported | Display enum has no Contents variant at src/style/computed.rs:27-3 |
| flex items are blockified regardless of child display:inline | https://www.w3.org/TR/css-di | supported-untested | src/layout/flex.rs:614-637 collects element children as flex items |
| float and clear have no effect on flex items | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:1206-1207 and 3274-3275 emit flex items with Fl |
| vertical-align has no effect on flex items | https://www.w3.org/TR/css-fl | supported-untested | FlexCell cross-axis placement uses align-items/align-self in src/l |
| visibility:collapse on flex items removes main-axis particip | https://www.w3.org/TR/css-fl | unsupported | Visibility::Collapse exists at src/style/computed.rs:579-585, but  |
| row-reverse main-start/main-end are logical and depend on di | https://www.w3.org/TR/css-fl | partial | src/layout/flex.rs:2701-2709 mirrors row-reverse physically and do |
| writing-mode maps flex row and column axes to vertical or ho | https://www.w3.org/TR/css-fl | partial | src/layout/flex.rs:1338-1403 treats row as physical horizontal and |
| align-content has no effect on a single-line flex container | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:1741-1802 applies align-content only when line_ |
| Box Alignment first baseline and last baseline keywords for  | https://www.w3.org/TR/css-al | unsupported | src/style/computed.rs:3228-3261 recognizes only the single-token b |
| safe and unsafe overflow-position prefixes on align-items, a | https://www.w3.org/TR/css-al | unsupported | src/style/computed.rs:3199-3210 strips safe/unsafe only for justif |
| place-content shorthand applies to flex container content di | https://www.w3.org/TR/css-al | unsupported | src/style/computed.rs:3462-3500 implements place-items/place-self  |
| place-items shorthand sets flex align-items | https://www.w3.org/TR/css-al | unsupported | src/style/computed.rs:3462-3470 maps place-items only to grid_alig |
| place-self shorthand sets flex align-self | https://www.w3.org/TR/css-al | unsupported | src/style/computed.rs:3485-3500 maps place-self only to grid self- |
| legacy grid-gap aliases apply as gap aliases in flex layout | https://www.w3.org/TR/css-al | supported-untested | src/style/computed.rs:3552-3557 maps grid-gap into row_gap/column_ |
| gap, row-gap, and column-gap accept calc() length-percentage | https://www.w3.org/TR/css-al | unsupported | parser can produce CssValue::Calc, but src/style/computed.rs:3384- |
| percentage row-gap in wrapped flex resolves against definite | https://www.w3.org/TR/css-al | supported-untested | src/layout/flex.rs:1351-1367 resolves row_gap_pct from style.heigh |
| flex-basis intrinsic keywords min-content and fit-content | https://www.w3.org/TR/css-fl | unsupported | src/style/computed.rs:3287-3320 handles auto/content/length/percen |
| flex-basis calc() length-percentage values | https://www.w3.org/TR/css-fl | unsupported | parse_length can return CssValue::Calc, but src/style/computed.rs: |
| flex shorthand basis-only and grow-plus-basis forms | https://www.w3.org/TR/css-fl | unsupported | src/style/computed.rs:3322-3378 only accepts none/auto/initial or  |
| order property accepts only integers and invalid decimal dec | https://www.w3.org/TR/css-di | partial | src/style/computed.rs:3264-3273 accepts CssValue::Length and casts |
| order-modified document order controls flex item painting wh | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:1332-1336 sorts items by order and src/layout/f |
| absolutely positioned flex children paint relative to order- | https://www.w3.org/TR/css-di | partial | src/layout/flex.rs:3465-3468 appends all abspos children after in- |
| column flex containers distribute flex-grow and flex-shrink  | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:2767-2872 implements grow/shrink for column dir |
| justify-content distributes free space on the column main ax | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:2875-2933 handles column main-axis justify-cont |
| align-items aligns column flex items on the horizontal cross | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:3107-3138 computes column cross-axis x offsets  |
| row-gap is the main-axis gap and column-gap the line/cross g | https://www.w3.org/TR/css-al | supported-untested | src/layout/flex.rs:1369-1377 swaps main_gap/line_gap for column di |
| flex container intrinsic sizing with width:min-content/max-c | https://www.w3.org/TR/css-fl | unsupported | src/style/computed.rs tracks intrinsic width keywords for blocks,  |
| aspect-ratio transfers a definite cross size into an auto fl | https://www.w3.org/TR/css-fl | partial | src/layout/flex.rs:1151-1166 computes aspect-ratio height from wid |
| overflow-wrap:anywhere lowers the automatic minimum main siz | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:1094-1099 suppresses the auto min-content floor |
| main-axis auto margins do not absorb negative free space whe | https://www.w3.org/TR/css-fl | supported-untested | src/layout/flex.rs:2305-2326 enables auto main margins only when f |
| automatic minimum main size for column flex items uses min-h | https://www.w3.org/TR/css-fl | unsupported | src/layout/flex.rs:1084-1118 implements auto minimum only for row/ |
| break-before and break-after forced page breaks apply to fle | https://www.w3.org/TR/css-br | unsupported | src/style/computed.rs:3559-3590 parses break-before/after, but src |

### fragmentation-paged
_The current target manifests already cover the core happy paths for page breaks, break-inside avoidance, widows/orphans, basic named-page geometry, first/left/right page margins, page/root backgrounds, basic footnotes, basic running elements, table-row splitting, multicol count/width/gap/rule/span/fill, and multicol pagination. The blind spots are concentrated in boundary-order semantics, side/selected page margin boxes, :blank pages, table header/footer repetition, flex/grid fragmentation, column-specific breaks and percentage gaps, column-rule suppression beside empty columns, and deeper GCPM behavior._

| feature | spec | status | evidence |
|---|---|---|---|
| page fragmentainers and pagination of normal-flow block cont | CSS Fragmentation Level 3; C | supported-tested | paged-media manifest: paged-forced-break-two-pages, break-before-p |
| break-before/break-after: page and legacy page-break-before/ | CSS Fragmentation Level 3 §4 | supported-tested | paged-media manifest: break-before-page-modern-real, break-after-p |
| break-before/break-after: left, right, recto, verso parity b | CSS Fragmentation Level 3 §4 | supported-untested | src/style/computed.rs BreakValue supports Left/Right/Recto/Verso a |
| forced break precedence when break-after and break-before me | CSS Fragmentation Level 3 §4 | partial | src/layout/engine.rs emits separate LayoutElement::PageBreak entri |
| break-before/break-after: avoid and avoid-page suppression o | CSS Fragmentation Level 3 §4 | partial | src/style/computed.rs parses avoid/avoid-page, but source evidence |
| break-before/break-after: column and avoid-column in multico | CSS Fragmentation Level 3 §4 | unsupported | src/style/computed.rs BreakValue::from_keyword lacks column and av |
| break-inside: avoid and avoid-page for page fragmentation of | CSS Fragmentation Level 3 §4 | supported-tested | paged-media manifest: paged-break-inside-avoid, break-inside-avoid |
| break-inside: avoid-column inside multicol with column-fill: | CSS Fragmentation Level 3 §4 | partial | src/style/computed.rs collapses avoid-column into break_inside_avo |
| legacy page-break-inside: avoid alias | CSS Fragmentation Level 3 §3 | supported-tested | paged-media manifest: page-break-inside-avoid-table-straddle and p |
| orphans and widows line-count constraints | CSS Fragmentation Level 3 §3 | supported-tested | paged-media manifest: orphans-widows-default, orphans-3, orphans-4 |
| box-decoration-break: slice and clone at page fragment bound | CSS Fragmentation Level 3 §5 | supported-tested | src/style/computed.rs parses BoxDecorationBreak::Slice/Clone and s |
| fragmented margins, borders, backgrounds and padding on spli | CSS Fragmentation Level 3 §5 | supported-tested | src/layout/paginate.rs split_container and split_text_block create |
| fragmenting raster images/replaced boxes across pages | CSS Fragmentation Level 3 §5 | partial | src/layout/paginate.rs split_image_block only splits object-fit:fi |
| fragmenting table rows/cells taller than a page | CSS Fragmentation Level 3 §5 | supported-tested | paged-media manifest: table-row-taller-than-page; src/layout/pagin |
| repeating table header and footer groups on each page fragme | CSS Fragmentation Level 3 ta | supported-untested | src/layout/paginate.rs tracks pending_table_headers and pending_ta |
| fragmenting flex containers and flex items across pages | CSS Fragmentation Level 3 §5 | partial | src/layout/paginate.rs split_element does not split LayoutElement: |
| fragmenting grid containers and grid items across pages | CSS Fragmentation Level 3 §5 | partial | src/layout/paginate.rs split_element does not split LayoutElement: |
| @page size descriptor with explicit width/height lengths | CSS Paged Media Level 3 §7.1 | supported-tested | all target fixtures use @page{size:<W>px <H>px}; src/parser/css/pa |
| @page size descriptor common page-size keywords and orientat | CSS Paged Media Level 3 §7.1 | partial | src/parser/css/page.rs supports A3/A4/A5/B5/letter/legal and orien |
| @page margin shorthand and margin-* longhands | CSS Paged Media Level 3 §6 a | partial | src/parser/css/page.rs supports one-, two-, and four-value margin  |
| @page background painting over the page box/bleed area | CSS Paged Media Level 3 §3.2 | supported-tested | paged-media manifest: at-page-background-bleed; src/parser/css/pag |
| root/canvas background behavior in paged output | CSS Paged Media Level 3 §3 a | supported-tested | paged-media manifest: root-background-content-box |
| @page :first selector | CSS Paged Media Level 3 §4.2 | supported-tested | paged-media manifest: paged-first-page-margin; src/parser/css/page |
| @page :left and :right spread selectors | CSS Paged Media Level 3 §4.2 | supported-tested | paged-media manifest: paged-spread-left-right-margins; src/parser/ |
| @page :blank selector for blank pages inserted by forced bre | CSS Paged Media Level 3 §4.2 | unsupported | src/parser/css/page.rs recognizes PageSelector::Blank, but src/lib |
| named pages via the page property, including named page marg | CSS Paged Media Level 3 §8 | supported-tested | paged-media manifest: paged-named-page, paged-named-page-margin, p |
| page selector lists and named-page plus pseudo-page combinat | CSS Paged Media Level 3 §4.3 | unsupported | src/parser/css/page.rs classify_page_selector returns one PageSele |
| page margin boxes in top and bottom bands with literal conte | CSS Paged Media Level 3 §5.2 | partial | src/parser/css/page.rs recognizes all margin box names but src/ren |
| page margin boxes in left/right side bands | CSS Paged Media Level 3 §5.2 | unsupported | src/parser/css/page.rs parses @left-* and @right-* margin boxes, b |
| selected and named @page margin boxes | CSS Paged Media Level 3 §5.1 | unsupported | src/lib.rs collects margin boxes only from page rules with selecto |
| page counters counter(page) and counter(pages) in page-margi | CSS Paged Media Level 3 §6.1 | supported-tested | src/parser/css/page.rs parse_margin_box_content handles counter(pa |
| counter-reset/counter-increment for page counters in @page | CSS Paged Media Level 3 §6.1 | unsupported | source searches show page margin box rendering uses raw page_num/t |
| column-count, column-width and columns shorthand column coun | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-count-three, multicol-column-wi |
| column-gap: normal and fixed lengths | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-gap, multicol-column-gap-normal |
| column-gap percentages | CSS Multi-column Layout Leve | unsupported | src/style/computed.rs stores column_gap_pct, but src/layout/multic |
| column-rule width/style/color and shorthand | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-rule-solid, multicol-column-rul |
| column rules are drawn only between adjacent columns that bo | CSS Multi-column Layout Leve | partial | src/layout/multicol.rs emits rule spans for column gaps based on c |
| column-span: all spanners interrupt multicol flow | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-span-all; paged-media manifest: |
| column-fill: balance | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-fill-balance and several defaul |
| column-fill: auto with definite height and pagination | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-column-fill-auto, multicol-page-break- |
| multicol pagination across page fragments with rules/gaps pr | CSS Multi-column Layout Leve | supported-tested | multicol manifest: multicol-page-break-flow, multicol-three-cols-p |
| column balancing with spanners and page boundaries | CSS Multi-column Layout Leve | supported-tested | paged-media manifest: multicol-span-all-page-break; multicol manif |
| float: footnote and basic footnote area placement | CSS Generated Content for Pa | supported-tested | paged-media manifest: footnote-float with oracle weasyprint; src/s |
| @footnote styling, ::footnote-call and ::footnote-marker | CSS Generated Content for Pa | partial | basic float:footnote exists, but source searches do not show @foot |
| footnote-display: block, inline, compact | CSS Generated Content for Pa | unsupported | source searches show no footnote-display parser or layout branch;  |
| footnote-policy: auto, line, block | CSS Generated Content for Pa | unsupported | source searches show no footnote-policy parser or pagination behav |
| running elements with position: running(name) and content: e | CSS Generated Content for Pa | supported-tested | paged-media manifest: running-element-header with oracle weasyprin |
| element(name, first/start/last/first-except) running-element | CSS Generated Content for Pa | unsupported | src/parser/css/page.rs parse_margin_box_content only handles eleme |
| named strings: string-set and string() in page-margin boxes | CSS Generated Content for Pa | unsupported | source searches found no string-set support; src/parser/css/page.r |
| running headers sourced from selected or named pages | CSS Paged Media Level 3 page | partial | running elements can render in unselected @page margin boxes, but  |

### grid
_Current grid parity coverage is strong for basic display:grid, fixed/percent/fr columns, fixed rows, integer repeat(), simple minmax(.,1fr), single auto columns, row/column gap lengths, default row auto-placement, grid-auto-flow:column, numeric spans, positive line placement, named lines, named areas, dot cells, and three basic item-alignment cases. It is thin or blind for intrinsic/fit-content track sizing, auto-repeat, implicit columns, grid shorthands/longhands, order and overlap painting, subgrid, alignment distribution/self/baseline edge cases, and grid fragmentation._

| feature | spec | status | evidence |
|---|---|---|---|
| display:grid establishes a block-level grid formatting conte | CSS Grid 1 section 2, grid c | supported-tested | manifest grid-display-grid; src/style/computed.rs:3177 maps displa |
| display:inline-grid establishes an inline-level grid contain | CSS Grid 1 section 2, grid c | unsupported | src/style/computed.rs:27-33 Display has Grid but no InlineGrid; no |
| grid items are laid out into grid cells and blockified as gr | CSS Grid 1 sections 6 and 10 | partial | src/layout/grid.rs:928-997 filters element children and absolute c |
| fixed <length> track sizing for columns | CSS Grid 1 section 7.2 track | supported-tested | manifest grid-display-grid, grid-template-columns-repeat; src/styl |
| fixed <length> track sizing for rows | CSS Grid 1 section 7.2 track | supported-tested | manifest grid-template-rows; src/layout/grid.rs:1051-1059 applies  |
| percentage track sizing in grid-template-columns | CSS Grid 1 section 7.2 track | supported-tested | manifest grid-template-columns-percent; src/style/computed.rs:6872 |
| fr flexible track sizing in columns | CSS Grid 1 sections 7.2 and  | supported-tested | manifest grid-template-columns-fr-mix; src/layout/grid.rs:80-178 i |
| fr flexible track sizing in rows | CSS Grid 1 sections 7.2 and  | partial | src/layout/grid.rs:214-221 says fr/auto/minmax rows fall back to a |
| auto track sizing for columns | CSS Grid 1 sections 7.2 and  | partial | manifest grid-template-columns-auto covers leftover auto; src/layo |
| auto rows and default row sizing | CSS Grid 1 sections 7.2, 7.6 | partial | src/layout/grid.rs:1051-1111 uses fixed/grid-auto/content fallback |
| minmax(min,max) with flexible max | CSS Grid 1 section 7.2.1 min | supported-tested | manifest grid-template-columns-minmax; src/style/computed.rs:6885- |
| minmax() with fixed max caps and non-flex max sizing | CSS Grid 1 section 7.2.1 min | partial | src/style/computed.rs:6902-6913 stores fixed max, but src/layout/g |
| minmax(auto,...) and minmax(...,auto) intrinsic semantics | CSS Grid 1 section 7.2.1 min | partial | src/style/computed.rs:6892-6904 coerces auto min to 0 and auto max |
| min-content track sizing keyword | CSS Grid 1 section 7.2.1 tra | partial | src/style/computed.rs:7029-7031 approximates min-content as GridTr |
| max-content track sizing keyword | CSS Grid 1 section 7.2.1 tra | partial | src/style/computed.rs:7029-7031 approximates max-content as GridTr |
| fit-content(<length-percentage>) track sizing function | CSS Grid 1 section 7.2.1 fit | partial | src/style/computed.rs:7014-7019 parses fit-content() as Auto and e |
| repeat(<integer>, <track-list>) fixed repeat notation | CSS Grid 1 section 7.2.3 rep | supported-tested | manifest grid-template-columns-repeat; src/style/computed.rs:6963- |
| repeat() with multiple tracks in the repeated pattern | CSS Grid 1 section 7.2.3 rep | supported-untested | src/style/computed.rs:6981-6995 recursively expands a track list p |
| repeat() with bracketed line names across repetitions | CSS Grid 1 section 7.2.3 rep | partial | src/style/computed.rs:6981-6995 merges line names, but placement r |
| repeat(auto-fill, ...) repeat-to-fill | CSS Grid 1 section 7.2.3.2 a | partial | src/style/computed.rs:6974-6979 hard-codes auto-fill to 3 repetiti |
| repeat(auto-fit, ...) repeat-to-fill with empty-track collap | CSS Grid 1 section 7.2.3.2 a | partial | src/style/computed.rs:6974-6979 hard-codes auto-fit to 3 repetitio |
| subgrid value for grid-template-columns/grid-template-rows | CSS Grid 2 sections 3.4 and  | unsupported | Grid 2 CRD and MDN Subgrid page fetched; src/style/computed.rs:686 |
| subgrid line-name augmentation and subgrid repeat(auto-fill) | CSS Grid 2 sections 7.2.6.2  | unsupported | no subgrid model in ComputedStyle fields at src/style/computed.rs: |
| grid-template-areas named rectangular areas | CSS Grid 1 section 7.3 named | supported-tested | manifest grid-template-areas-basic and grid-area-span-rows; src/st |
| grid-template-areas null cells using dot tokens | CSS Grid 1 section 7.3 named | supported-tested | manifest grid-template-areas-dot; src/style/computed.rs:6834-6842  |
| grid-template-areas invalid non-rectangular/disconnected are | CSS Grid 1 section 7.3 named | partial | src/style/computed.rs:6819-6864 pads rows and does not validate re |
| implicit line names generated by grid-template-areas (<area> | CSS Grid 1 section 7.3.2 imp | supported-untested | src/layout/grid.rs:541-566 derives area start/end lines; no manife |
| implicit named areas generated by explicit foo-start/foo-end | CSS Grid 1 section 7.3.3 imp | supported-untested | grid-area:<name> resolves through named <name>-start/end in src/la |
| grid-template shorthand | CSS Grid 1 section 7.4 expli | unsupported | src/parser/css/values.rs:461 preserves grid-template but src/style |
| grid shorthand including auto-flow syntax and reset semantic | CSS Grid 1 section 7.8 grid  | unsupported | no parser/computed handling for property grid in rg results; no ma |
| grid-auto-rows single fixed implicit row size | CSS Grid 1 section 7.6 impli | supported-tested | manifest grid-implicit-tracks; src/style/computed.rs:3445-3454 par |
| grid-auto-rows multiple-size repeating pattern | CSS Grid 1 section 7.6 impli | unsupported | ComputedStyle has grid_auto_rows: Option<f32> only at src/style/co |
| grid-auto-columns implicit column sizing | CSS Grid 1 section 7.6 impli | unsupported | no grid-auto-columns field or parser handling; src/layout/grid.rs: |
| implicit rows created by auto-placement overflow | CSS Grid 1 sections 7.5 and  | supported-tested | manifest grid-implicit-tracks; src/layout/grid.rs:696-725 grows oc |
| implicit columns created by placement outside explicit grid | CSS Grid 1 sections 7.5 and  | partial | src/layout/grid.rs:841 updates max_cols, but src/layout/grid.rs:10 |
| grid-auto-flow: row sparse auto-placement | CSS Grid 1 section 8.5 auto- | supported-tested | manifest grid-display-grid and span fixtures exercise default row  |
| grid-auto-flow: column auto-placement | CSS Grid 1 sections 7.7 and  | supported-tested | manifest grid-auto-flow-column; src/layout/grid.rs:758-807 impleme |
| grid-auto-flow: dense backfilling | CSS Grid 1 sections 7.7 and  | supported-untested | src/style/computed.rs:3455-3458 parses dense; src/layout/grid.rs:7 |
| auto-placement of items with definite row or definite column | CSS Grid 1 section 8.5 auto- | partial | src/layout/grid.rs:681-693 resolves axes independently and src/lay |
| order property affects grid auto-placement and painting orde | CSS Grid 1 sections 6.3 and  | unsupported | ComputedStyle stores order at src/style/computed.rs:1384-1386, but |
| grid-column and grid-row shorthands | CSS Grid 1 section 8.4 place | supported-tested | manifest grid-column-span, grid-row-span, grid-column-line-numbers |
| grid-column-start/grid-column-end/grid-row-start/grid-row-en | CSS Grid 1 section 8.3 line- | unsupported | src/parser/css/values.rs does not preserve grid-column-start/end o |
| positive integer line placement | CSS Grid 1 section 8.3 line- | supported-tested | manifest grid-column-line-numbers and grid-row-line-numbers; src/l |
| negative integer line placement from the end edge | CSS Grid 1 section 8.3 line- | supported-untested | src/layout/grid.rs:583-586 resolves negative line numbers; no pari |
| named line placement by explicit bracketed line names | CSS Grid 1 section 8.3 line- | supported-tested | manifest grid-named-lines-basic and grid-named-line-placement; src |
| repeated named lines with nth occurrence syntax (<integer> & | CSS Grid 1 section 8.3 grid- | unsupported | src/style/computed.rs:6761-6783 parse_grid_line accepts either int |
| numeric span placement (span N) | CSS Grid 1 section 8.3 grid- | supported-tested | manifest grid-column-span, grid-row-span, grid-column-span-to-line |
| named span placement (span <custom-ident>) | CSS Grid 1 section 8.3 grid- | partial | src/style/computed.rs:6771-6772 parses SpanNamed but src/layout/gr |
| grid-area single named area placement | CSS Grid 1 section 8.4 grid- | supported-tested | manifest grid-template-areas-basic, grid-template-areas-dot, grid- |
| grid-area four-line form row-start/col-start/row-end/col-end | CSS Grid 1 section 8.4 grid- | supported-tested | manifest grid-area-line-form; src/style/computed.rs:6798-6809 pars |
| overlapping grid areas and z-index stacking for static grid  | CSS Grid 1 section 6.5 z-axi | unsupported | src/layout/grid.rs:1163-1170 skips later overlapping cells; TableC |
| absolutely positioned children of a grid container are not g | CSS Grid 1 section 10 absolu | supported-untested | src/layout/grid.rs:982-997 filters absolute children out of grid i |
| row-gap, column-gap, and gap on grids | CSS Grid 1 section 10.1 gutt | supported-tested | manifest grid-gap; src/layout/grid.rs:921-922 reads row_gap/column |
| legacy grid-gap/grid-row-gap/grid-column-gap aliases | CSS Align 3 gutter aliases r | supported-untested | src/parser/css/inline.rs:632-635 and src/parser/css/values.rs:419- |
| percentage gaps in grid layout | CSS Align 3 gap percentage b | partial | src/style/computed.rs has parser unit grid_gap_from_percentage but |
| justify-items item alignment start/end/center/stretch | CSS Grid 1 section 10.3 inli | supported-tested | manifest grid-justify-items-end and grid-place-items-center; src/l |
| align-items item alignment start/end/center/stretch | CSS Grid 1 section 10.4 bloc | supported-tested | manifest grid-align-items-start and grid-place-items-center; src/s |
| place-items shorthand | CSS Grid 1 alignment section | supported-tested | manifest grid-place-items-center; src/style/computed.rs:3462-3470  |
| justify-self/align-self/place-self on individual grid items | CSS Grid 1 sections 10.3 and | supported-untested | src/style/computed.rs:3472-3499 parses self-alignment; src/layout/ |
| baseline alignment for grid items | CSS Grid 1 section 10 alignm | unsupported | GridAlign enum at src/style/computed.rs:157-169 has only stretch/s |
| justify-content and align-content distribution of the grid w | CSS Grid 1 section 10.5 alig | partial | src/style/computed.rs:3199-3250 parses flex/content alignment fiel |
| auto margins on grid items for alignment | CSS Grid 1 section 10.2 alig | unsupported | ComputedStyle tracks auto margins for block/flex, but src/layout/g |
| fragmentation between grid rows in paged media | CSS Grid 1 section 13 fragme | supported-untested | grid rows are emitted as Container children at src/layout/grid.rs: |
| fragmentation inside a grid item or across a spanned grid ar | CSS Grid 1 section 13 fragme | partial | src/layout/grid.rs:1147-1152 says multi-row items are approximated |
| CSSOM resolved value serialization of grid-template-* track  | CSS Grid 1 section 7.2.6 and | na-not-pdf-relevant | CSSOM serialization has no visual effect in a static PDF without s |

### tables
_The current tables manifest strongly covers ordinary HTML table grids, positive rowspan/colspan, simple separated and collapsed borders, one/two-value border-spacing, fixed layout with colgroup/percentage/remainder cases, basic auto layout, td/th padding and alignment defaults, top/bottom captions, empty whitespace cells, baseline/top/middle/bottom vertical alignment, row-group ordering, and multipage header/footer repetition. The main blind spots are spec-level table fixup/display roles, collapsed-border conflict winners, subtle auto/fixed layout interactions, empty-cell visibility semantics, zero-rowspan, multiple captions, and combined-property edge cases._

| feature | spec | status | evidence |
|---|---|---|---|
| HTML table element establishes the table grid and table wrap | CSS Tables 3 §2.1, CSS 2.2 § | supported-tested | manifest tables-basic-grid; src/layout/engine.rs:2554-2567 dispatc |
| CSS display: table and inline-table create table formatting  | MDN display table/internal v | unsupported | Display enum omits table/inline-table in src/style/computed.rs:25- |
| CSS internal table display values: table-row-group, table-he | MDN display table/internal v | unsupported | Computed display has no internal table roles; layout dispatch is t |
| Anonymous table fixup for missing parents/children around ta | CSS Tables 3 §2.2 and §2.2.1 | unsupported | src/layout/table.rs:608-670 only consumes actual tr/thead/tbody/tf |
| Missing-cells fixup appends anonymous cells so every row cov | CSS Tables 3 §3.4 | partial | src/layout/table.rs:1384-1390 breaks when a short row has no next  |
| Table row groups thead/tbody/tfoot participate in the row gr | CSS Tables 3 §3.2; CSS 2.2 § | supported-tested | manifest tables-thead-tbody-tfoot and multipage-tfoot-before-tbody |
| Table headers and footers repeat across page breaks in paged | CSS Tables 3 table-header-gr | supported-tested | manifest multipage-thead-repeat, multipage-tfoot-repeat, multipage |
| colgroup and col elements contribute column widths in fixed  | CSS 2.2 §17.5.2.1; CSS Table | supported-tested | manifest tables-layout-fixed; src/layout/table.rs:735-823 collects |
| colgroup/col span attributes and bare colgroup columns | CSS Tables 3 §3.3 table-colu | supported-untested | src/layout/table.rs:735-823 handles col span and bare colgroup spa |
| HTML colspan distributes a cell across multiple columns | CSS Tables 3 §3.3.1 and §3.8 | supported-tested | manifest tables-colspan and tables-colspan-rowspan; src/layout/tab |
| HTML rowspan distributes a cell across multiple rows | CSS Tables 3 §3.3.1 and §3.8 | supported-tested | manifest tables-rowspan, tables-rowspan-stagger, tables-colspan-ro |
| HTML rowspan=0 spans the remaining rows in the row group | HTML table model as referenc | unsupported | src/layout/table.rs:1510-1513 parses rowspan then clamps with max( |
| table-layout: fixed uses table/column/first-row widths and i | CSS 2.2 §17.5.2.1; CSS Table | supported-tested | manifest tables-layout-fixed, tables-width-percent-columns, tables |
| table-layout: fixed cell overflow is controlled by the cell  | CSS 2.2 §17.5.2.1 | partial | table cells carry content boxes but src/render/pdf/layout_elements |
| table-layout: auto computes min/preferred widths from non-sp | CSS 2.2 §17.5.2.2; CSS Table | supported-tested | manifest tables-layout-auto; src/layout/table.rs:1050-1160 compute |
| table-layout: auto may grow past a declared width when nowra | CSS 2.2 §17.5.2.2; CSS Table | partial | manifest tables-layout-auto-overflow is expected_support partial a |
| table-layout: auto distributes spanning-cell min/max contrib | CSS Tables 3 §3.8.3; CSS 2.2 | partial | src/layout/table.rs:1092-1109 divides spanning-cell min/preferred  |
| border-collapse: separate paints independent cell borders | CSS 2.2 §17.6.1; CSS Tables  | supported-tested | manifest tables-border-separate and tables-border-spacing-zero; sr |
| border-collapse: collapse merges adjacent borders into share | CSS 2.2 §17.6.2; CSS Tables  | supported-tested | manifest tables-border-collapse covers equal adjacent collapsed bo |
| Collapsed border conflict resolution: hidden wins, none lose | CSS 2.2 §17.6.2.1 | partial | src/render/pdf.rs:2779-2879 paints each cell edge independently wi |
| Collapsed table outer border sizing and half-border placemen | CSS 2.2 §17.6.2 | partial | src/layout/table.rs:506-578 derives outer collapsed left/right fro |
| border-spacing accepts one or two nonnegative lengths in sep | CSS 2.2 §17.6.1; CSS Tables  | supported-tested | manifest tables-border-spacing, tables-border-spacing-two-value, t |
| border-spacing is ignored when border-collapse is collapse | CSS Tables 3 §3.5.2 and §3.5 | supported-untested | src/render/pdf/layout_elements.rs:282-299 and src/layout/table.rs: |
| empty-cells: show paints borders/backgrounds for empty cells | CSS 2.2 §17.6.1.1 | supported-tested | default show behavior is exercised by ordinary separated-table fix |
| empty-cells: hide suppresses empty-cell borders/backgrounds, | CSS 2.2 §17.6.1.1 | supported-tested | manifest tables-empty-cells-hide and tables-empty-cells-whitespace |
| empty-cells: hide treats visibility:hidden content as no vis | CSS 2.2 §17.6.1.1 | partial | src/layout/table.rs:96-108 checks only DOM text/element presence,  |
| vertical-align: top, middle, bottom align table cell content | CSS 2.2 §17.5.3 | supported-tested | manifest tables-cell-vertical-align; src/render/pdf/layout_element |
| vertical-align: baseline aligns baselines across cells and c | CSS 2.2 §17.5.3 | supported-tested | manifest tables-vertical-align-baseline and tables-vertical-align- |
| Non-cell vertical-align values on table cells (sub, super, t | CSS 2.2 §17.5.3 | partial | VerticalAlign enum omits length/percentage and src/render/pdf/layo |
| caption-side: top and bottom position table captions before/ | CSS Tables 3 §3.5.3; CSS 2.2 | supported-tested | manifest tables-caption, tables-caption-side-bottom, multipage-cap |
| Multiple table-caption boxes around one table are all laid o | CSS Tables 3 table-caption b | partial | src/layout/table.rs:618-632 stores only the first caption in an Op |
| display: table-caption creates captions from non-caption ele | MDN display table/internal v | unsupported | internal display values are not parsed or laid out; only HtmlTag:: |
| Default th presentation: bold, centered header cells with ta | HTML UA stylesheet behavior  | supported-tested | manifest tables-th-header and tables-th-default; defaults in src/s |
| Row, row-group, column, column-group, table, and cell backgr | CSS 2.2 §17.5.1 and §17.6.1; | partial | manifest covers row-group backgrounds only; source falls back from |
| visibility: collapse for table rows, columns, row groups, an | CSS 2.2 §17.5.5; CSS Tables  | partial | src/layout/table.rs:1229-1329 has a row-only collapsed-border spec |

### text-inline-fonts-generated
_The existing parity manifests cover the common horizontal text path well: line-height number/length, basic vertical-align keywords, normal/pre/nowrap/pre-wrap/pre-line white-space, text-align right/center/justify, letter/word spacing, text-indent length, simple case transforms, basic font families/sizes/weight/style, common list marker types and positions, ordered-list continuation across pages, basic counters, ::before/::after strings/attrs/urls/counters/quotes, ::first-line color/background, and ::first-letter color/transform/drop-cap. The blind spots are mostly value-space edges and cross-feature flows: percentage line metrics, length/percentage vertical-align, direction-sensitive logical alignment, source-only tab/writing-mode/font-variant/list-image support, richer OpenType controls, CJK/bidi/writing-mode details, marker content, counter-set/reversed counters, and geometry-affecting ::first-line styling._

| feature | spec | status | evidence |
|---|---|---|---|
| text-transform: none / uppercase / lowercase / capitalize | CSS Text 3 #propdef-text-tra | supported-tested | inline-text-text-transform-uppercase/lowercase/capitalize and font |
| text-transform: full-width / full-size-kana | CSS Text 3 #propdef-text-tra | na-not-pdf-relevant | Fetched CSS Text 3 marks both values at-risk in /tmp/css-text-3.fi |
| white-space: normal / nowrap / pre / pre-wrap / pre-line | CSS Text 3 #propdef-white-sp | supported-tested | inline-text and text-advanced manifest ids cover normal, nowrap, p |
| white-space: break-spaces | CSS Text 3 #propdef-white-sp | partial | inline-text-white-space-break-spaces implemented but text-advanced |
| white-space collapsing, segment breaks, trimming, preserved  | CSS Text 3 #white-space-proc | supported-tested | inline-text-white-space-normal/pre/pre-line and text-advanced-whit |
| tab-size: <number> | CSS Text 3 #propdef-tab-size | supported-untested | text-advanced-tab-size is expected_support unsupported, but src/st |
| tab-size: <length> | CSS Text 3 #propdef-tab-size | na-not-pdf-relevant | Fetched CSS Text 3 marks <length> tab-size at-risk in /tmp/css-tex |
| overflow-wrap: normal / break-word / anywhere and word-wrap  | CSS Text 3 #overflow-wrap-pr | supported-tested | inline-text-overflow-wrap-break-word and text-advanced-overflow-wr |
| word-break: break-all | CSS Text 3 #word-break-prope | partial | inline-text-word-break-break-all implemented but text-advanced-wor |
| word-break: keep-all | CSS Text 3 #word-break-prope | partial | text-advanced-word-break-keep-all only covers spaced Latin text; n |
| line-break: auto / loose / normal / strict / anywhere | CSS Text 3 #line-break-prope | unsupported | No manifest entry; rg found no line-break handling under src/ |
| hyphens: none / manual / auto | CSS Text 3 #hyphens-property | unsupported | text-advanced-hyphens-auto expected_support unsupported; rg finds  |
| soft wrap opportunities after hyphen-minus inside words | CSS Text 3 #line-breaking | supported-untested | No parity manifest id; src/layout/text.rs:429-436 documents and im |
| text-align: left / right / center / justify | CSS Text 3 #text-align-prope | supported-tested | inline-text-text-align-right/center/justify and justify-multiline; |
| text-align: start / end / match-parent | CSS Text 3 #text-align-prope | unsupported | No manifest entry; src/style/computed.rs:3137-3144 only recognizes |
| text-align-last | CSS Text 3 #propdef-text-ali | unsupported | No manifest entry; rg found no text-align-last handling under src/ |
| text-justify | CSS Text 3 #propdef-text-jus | na-not-pdf-relevant | Fetched CSS Text 3 marks text-justify at-risk in /tmp/css-text-3.f |
| letter-spacing: normal / <length>, including negative length | CSS Text 3 #letter-spacing-p | supported-tested | inline-text-letter-spacing and inline-text-letter-spacing-negative |
| word-spacing: normal / <length>, including negative length | CSS Text 3 #word-spacing-pro | supported-tested | inline-text-word-spacing and inline-text-word-spacing-negative; sr |
| word-spacing: <percentage> | CSS Text 3 #word-spacing-pro | unsupported | No manifest entry; src/style/computed.rs:4487-4489 only consumes C |
| text-indent: <length> | CSS Text 3 #text-indent-prop | supported-tested | inline-text-text-indent-length and text-advanced-text-indent; src/ |
| text-indent: <percentage> | CSS Text 3 #text-indent-prop | unsupported | No manifest entry; src/style/computed.rs:4450-4452 only consumes C |
| text-indent: hanging / each-line | CSS Text 3 #text-indent-prop | na-not-pdf-relevant | Not included in stable gap set because interoperable browser suppo |
| text-overflow: clip / ellipsis | CSS Overflow 3 #propdef-text | partial | text-advanced-text-overflow-ellipsis/clip expected_support partial |
| text-overflow: <string> | CSS Overflow 3 #propdef-text | unsupported | text-advanced-text-overflow-string expected_support unsupported; s |
| line-height: normal / <number> / <length> | CSS Inline 3 #propdef-line-h | supported-tested | typography-line-height-numeric/length and inline-text-line-height- |
| line-height: <percentage> | CSS Inline 3 #propdef-line-h | unsupported | No manifest entry; parser can produce Percentage but src/style/com |
| mixed inline font sizes contributing to line-box height and  | CSS Inline 3 #inline-height | supported-untested | No direct parity id for mixed font-size leading; src/layout/text.r |
| vertical-align keyword baseline / sub / super / top / middle | CSS Inline 3 #propdef-vertic | supported-tested | inline-text vertical-align fixtures and typography sub/sup; src/st |
| vertical-align: text-bottom | CSS Inline 3 #propdef-vertic | supported-untested | No manifest id for text-bottom; src/style/computed.rs:4501-4505 an |
| vertical-align: <length> / <percentage> | CSS Inline 3 #propdef-vertic | unsupported | No manifest entry; src/style/computed.rs:4492-4505 only consumes C |
| inline-block baseline, last-line baseline, shrink-to-fit inl | CSS Inline 3 / CSS2 inline f | supported-tested | inline-text-inline-block-baseline, baseline-multiline, shrink-to-f |
| alignment-baseline, baseline-source, baseline-shift longhand | CSS Inline 3 #baseline-shift | na-not-pdf-relevant | CSS Inline 3 is WD and these longhands are not broadly interoperab |
| initial-letter property | CSS Inline 3 #propdef-initia | unsupported | No manifest entry; rg found no initial-letter property handling un |
| font-family generic serif / sans-serif / monospace and fallb | CSS Fonts 4 #font-family-pro | supported-tested | typography-font-family-serif/sans-serif/monospace; src/style/compu |
| @font-face font-family + src:url() local/relative font regis | CSS Fonts 4 #font-face-rule | partial | fonts-advanced-font-face-custom-src expected_support partial; src/ |
| @font-face descriptors beyond family/src: font-weight, font- | CSS Fonts 4 #font-face-rule | unsupported | No target manifest coverage; src/parser/css/page.rs:50-83 ignores  |
| font-size: px / pt / em / rem / percentage | CSS Fonts 4 #font-size-prop | supported-tested | typography-font-size-px/pt/em/rem/percent and fonts-advanced font- |
| font-size: ex / ch | CSS Fonts 4 / CSS Values fon | supported-untested | fonts-advanced-font-size-ex/ch expected_support unsupported, but s |
| font-weight: normal / bold | CSS Fonts 4 #font-weight-pro | supported-tested | typography-font-weight-bold/normal; src/style/computed.rs:2970-297 |
| font-weight numeric 1-1000 and relative bolder/lighter | CSS Fonts 4 #font-weight-pro | partial | No parity fixture for numeric ladder; src/style/computed.rs:2970-2 |
| font-style: normal / italic / oblique | CSS Fonts 4 #font-style-prop | partial | typography-font-style-italic tests italic; src/style/computed.rs:2 |
| font-stretch / font-width | CSS Fonts 4 #font-stretch-pr | unsupported | fonts-advanced-font-stretch-condensed expected_support unsupported |
| font-variant-caps / font-variant: small-caps | CSS Fonts 4 #font-variant-ca | supported-untested | fonts-advanced-font-variant-small-caps expected_support unsupporte |
| font-feature-settings OpenType feature tags, especially liga | CSS Fonts 4 #font-feature-se | partial | fonts-advanced-font-feature-settings-ligatures expected_support un |
| font-variant-ligatures | CSS Fonts 4 #font-variant-li | unsupported | No manifest entry; rg found no font-variant-ligatures handling und |
| font-variant-numeric | CSS Fonts 4 #font-variant-nu | unsupported | No manifest entry; rg found no font-variant-numeric handling under |
| font-variant-east-asian | CSS Fonts 4 #font-variant-ea | unsupported | No manifest entry; rg found no font-variant-east-asian handling un |
| font-kerning | CSS Fonts 4 #font-kerning-pr | unsupported | No manifest entry; rg found no font-kerning property handling unde |
| font-variation-settings and variable font axes | CSS Fonts 4 #font-variation- | unsupported | No manifest entry; rg found no font-variation-settings handling un |
| font-optical-sizing | CSS Fonts 4 #font-optical-si | unsupported | No manifest entry; rg found no font-optical-sizing handling under  |
| font-size-adjust | CSS Fonts 4 #font-size-adjus | unsupported | No target manifest entry; rg only finds unrelated SVG parser tests |
| font-synthesis property controlling faux bold/italic/small-c | CSS Fonts 4 #font-synthesis- | unsupported | Source has internal faux oblique/bold comments, but rg found no fo |
| font shorthand | CSS Fonts 4 #font-prop | unsupported | No target manifest entry; rg found no font shorthand decoder for H |
| direction: ltr / rtl and dir attribute inheritance | CSS Writing Modes 4 #directi | partial | text-advanced-direction-ltr implemented and direction-rtl expected |
| unicode-bidi: normal / bidi-override / isolate-override | CSS Writing Modes 4 #unicode | partial | text-advanced-unicode-bidi-override expected_support unsupported;  |
| mixed-script Unicode bidi reordering without explicit unicod | CSS Writing Modes 4 #text-di | supported-untested | No target parity id for mixed-script visual order; src/layout/text |
| writing-mode: horizontal-tb | CSS Writing Modes 4 #block-f | supported-tested | Default behavior exercised by all horizontal inline-text fixtures; |
| writing-mode: vertical-rl | CSS Writing Modes 4 #block-f | supported-untested | text-advanced-writing-mode-vertical-rl expected_support unsupporte |
| writing-mode: vertical-lr / sideways-rl / sideways-lr | CSS Writing Modes 4 #block-f | unsupported | No manifest entry; src/style/computed.rs:4400-4407 explicitly fall |
| text-orientation: mixed / upright / sideways | CSS Writing Modes 4 #text-or | unsupported | No manifest entry; rg found no text-orientation parser, and src/re |
| text-combine-upright | CSS Writing Modes 4 #text-co | unsupported | No manifest entry; rg found no text-combine-upright handling under |
| list-style-type: disc / circle / square / decimal / decimal- | CSS Lists 3 #text-markers | supported-tested | lists-counters manifest covers all listed built-in types; src/styl |
| list-style-type predefined counter styles beyond latin/roman | CSS Lists 3 / CSS Counter St | unsupported | No target manifest entry; src/style/computed.rs:5124-5138 unknown  |
| list-style-position: inside / outside | CSS Lists 3 #list-style-posi | supported-tested | list-style-position-inside/outside; src/layout/engine.rs:2769-3031 |
| list-style-image: url() | CSS Lists 3 #image-markers | supported-untested | list-style-image-data-uri expected_support unsupported, but src/st |
| list-style shorthand | CSS Lists 3 #list-style-prop | supported-tested | list-style-shorthand; src/style/computed.rs:5093-5107 decodes type |
| ::marker color and font styling | CSS Pseudo 4 #marker-pseudo | supported-tested | marker-pseudo-color; src/layout/engine.rs:2817-2875 resolves marke |
| ::marker { content: ... } overriding default marker | CSS Pseudo 4 #marker-pseudo | supported-untested | No manifest id for marker content; src/layout/engine.rs:2827-2846  |
| counter-reset and counter-increment | CSS Lists 3 #auto-numbering | supported-tested | counter-reset-increment and generated-content-counter; src/layout/ |
| counter-set | CSS Lists 3 #propdef-counter | unsupported | No manifest entry; rg found no counter-set handling under src/styl |
| counter() with explicit counter style | CSS Lists 3 #counter-functio | supported-tested | generated-content-counter-roman implemented; lists-counters counte |
| counters() nested counter chains and scope | CSS Lists 3 #counter-functio | supported-tested | generated-content-counters-nested implemented; lists-counters coun |
| reversed counters and reversed() counter-reset | CSS Lists 3 #reversed-counte | unsupported | No manifest entry; rg found no reversed counter handling under src |
| HTML ol start and nested ol restart behavior | CSS Lists 3 #ua-stylesheet | supported-tested | ol-start-attribute and ol-decimal-markers |
| HTML ol reversed and li value attributes | CSS Lists 3 / HTML list numb | unsupported | No target manifest entry; rg found ol start handling but no ol rev |
| content: normal / none on ::before/::after | CSS Content 3 #content-prope | supported-tested | generated-content-content-none and generated-content-content-norma |
| content string concatenation and attr() | CSS Content 3 #content-prope | supported-tested | generated-content-before-string/after-string/attr/attr-missing/con |
| content: url() replaced pseudo-element image | CSS Content 3 #content-prope | supported-tested | generated-content-content-url-image and content-url-sized; src/lay |
| content: counter() and counters() | CSS Content 3 #content-prope | supported-tested | generated-content-counter, generated-content-counters-nested, gene |
| content open-quote / close-quote / no-open-quote / no-close- | CSS Content 3 #quotes-proper | supported-tested | generated-content-open-close-quote, no-quote-keywords, nested-quot |
| content target-counter(), target-counters(), target-text(),  | CSS Content 3 | na-not-pdf-relevant | CSS Content 3 is WD and these generated-content functions are not  |
| ::before and ::after generated boxes: inline, block, inline- | CSS Pseudo 4 #generated-cont | supported-tested | generated-content-before-string/after-string/before-block/before-d |
| ::before/::after suppression on replaced elements | CSS Pseudo 4 #generated-cont | supported-tested | generated-content-after-replaced |
| ::first-line color/font-weight/background/text-decoration su | CSS Pseudo 4 #first-line-pse | supported-tested | generated-content-first-line and first-line-background; src/layout |
| ::first-line font-size and geometry-affecting line metrics | CSS Pseudo 4 #first-line-pse | partial | No parity fixture for first-line font-size; src/layout/helpers.rs: |
| ::first-letter color/transform and floated drop-cap | CSS Pseudo 4 #first-letter-p | supported-tested | generated-content-first-letter-color/transform/dropcap; src/layout |
| ::first-letter punctuation-including first-letter unit | CSS Pseudo 4 #application-in | supported-untested | No manifest id for leading quote/punctuation; src/layout/helpers.r |
| ::selection and highlight pseudo-elements | CSS Pseudo 4 #highlight-pseu | na-not-pdf-relevant | Interactive user selection/highlight state is excluded for static  |
| text-wrap shorthand and text-wrap-mode/text-wrap-style longh | CSS Text 4 #text-wrap-shorth | unsupported | Fetched CSS Text 4 exposes text-wrap-mode, text-wrap-style, and te |
| white-space-collapse longhand | CSS Text 4 #white-space-coll | unsupported | CSS Text 4 defines white-space-collapse values including preserve  |
| white-space-trim longhand | CSS Text 4 #white-space-trim | na-not-stable | Fetched CSS Text 4 defines white-space-trim, but it is a newer Tex |
| text-decoration-line multi-value grammar | CSS Text Decoration 3 #text- | unsupported | Only text-decoration single keywords set underline/line-through/ov |
| text-decoration-style | CSS Text Decoration 3 #text- | unsupported | Fetched Text Decoration 3 defines solid/double/dotted/dashed/wavy; |
| text-decoration-thickness and text-underline-offset | CSS Text Decoration 4 #text- | unsupported | Renderer uses fixed font-size-relative underline metrics; no parse |
| text-emphasis marks | CSS Text Decoration 3 #text- | unsupported | Fetched Text Decoration 3 defines text-emphasis; rg found no text- |
| text-shadow as text decoration | CSS Text Decoration 3 #text- | supported-untested-in-area | Effects manifest has text-shadow probes, while src/style/computed. |
| dir=auto HTML direction resolution | WHATWG HTML dir attribute /  | unsupported | Source handles dir='rtl' and dir='ltr' only; no dir='auto' content |
| unicode-bidi: plaintext/isolate/embed | CSS Writing Modes 4 #unicode | unsupported | Computed style has only a bidi_override boolean for bidi-override/ |
| font-width property alias for font-stretch | CSS Fonts 4 #font-stretch-pr | unsupported | CSS Fonts 4 uses font-width as the current name; source has no fon |
| @font-face size-adjust and metric override descriptors | CSS Fonts 4/5 #font-face-rul | unsupported | @font-face parser extracts only font-family and src, ignoring size |
| font shorthand system and font property reset behavior | CSS Fonts 4 #font-prop | unsupported | No font shorthand decoder was found; individual font-size/weight/s |
| font-kerning property | CSS Fonts 4 #font-kerning-pr | unsupported | Round 1 noted no font-kerning handling; ParitySans contains a kern |
| font-palette and color-font palette selection | CSS Fonts 4 #font-palette-pr | na-not-stable-no-fixture | Requires a deterministic color font/palette asset; no small no-ass |
| list-style-type:<string> | CSS Lists 3 #valdef-list-sty | unsupported | CSS Lists 3 allows string markers; parse_list_style_type falls unk |
| @counter-style custom counter styles | CSS Counter Styles 3 #counte | unsupported | Fetched Counter Styles 3 defines @counter-style; rg found no @coun |
| marker-side | CSS Lists 3 #propdef-marker- | unsupported | CSS Lists 3 defines marker-side match-self/match-parent; source ha |
| special list-item counter | CSS Lists 3 #list-item-count | unsupported | List marker numbering uses an internal ListContext index but does  |
| HTML ol reversed and li value integration | WHATWG HTML ol/li attributes | unsupported | Source handles ol start only; rg found no reversed ol or li value  |
| content: leader() | CSS Content 3 #leader-functi | unsupported | parse_content_value handles strings, attr, counter(s), url, quote  |
| target-counter(), target-counters(), target-text() | CSS Content 3 #target-counte | unsupported | Generated-content parser has no target-* functions; these are PDF- |
| string-set and string() running strings | CSS Content 3 #string-set /  | unsupported | No string-set property or string() generated-content function hand |
| content: contents keyword | CSS Content 3 #content-prope | unsupported | CSS Content 3 includes the contents keyword in <content-list>; par |
| ::before/::after display:list-item generated marker | CSS Pseudo 4 generated conte | unsupported | Pseudo helpers build inline/block/inline-block boxes but no list-i |
| expanded ::first-line applicable properties | CSS Pseudo 4 #first-line-pse | partial | apply_first_line_style copies color/font family/weight/style/decor |

### transforms-pos-overflow-images-units-box
_Current parity coverage is strong for ordinary 2D transform shorthand functions, transform-origin, basic absolute/relative/fixed positioning on a single page, z-index/source-order stacking, hidden/visible/scroll clipping, padding-box and border-radius clipping, basic raster/SVG image sizing, object-fit scale-down and common object-position forms, box-sizing, CSS 2.x margin collapse cases, percentage/absolute/calc/var units, flex/grid interactions, and common Selectors 4/cascade paths. The blind spots are mainly stale expected-unsupported fixtures that no longer lock positive behavior, print-specific fixed-position pagination, 3D transform semantics, Display 3 box-suppression/BFC values, far-edge object-position math, advanced selector/cascade features, and font/viewport/intrinsic sizing combinations._

| feature | spec | status | evidence |
|---|---|---|---|
| transform property with 2D transform-list composition order | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-translate, trans |
| translate(), translateX(), translateY(), including percentag | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-translate, trans |
| scale(), scaleX(), scaleY(), omitted second argument and neg | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-scale, transform |
| rotate() angle units deg/rad/turn and negative angles | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-rotate, transfor |
| skewX() and skewY() | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-skew-x, transfor |
| two-axis skew() shorthand | CSS Transforms 1 | supported-untested | tests/parity/manifest/transforms.json: transforms-skew is marked e |
| matrix(a,b,c,d,e,f) | CSS Transforms 1 | supported-untested | tests/parity/manifest/transforms.json: transforms-matrix is marked |
| transform-origin keywords, percentages, lengths, and default | CSS Transforms 1 | supported-tested | tests/parity/manifest/transforms.json: transforms-origin-center, t |
| transform-box reference box selection | CSS Transforms 1 | unsupported | No transform-box handling found under src/; TransformOrigin resolv |
| transformed element establishes containing block for absolut | CSS Transforms 1 | supported-tested | tests/parity/manifest/positioning.json: positioning-transformed-co |
| individual transform properties translate, rotate, scale and | CSS Transforms 2 | unsupported | No computed-style handling for CSS properties named translate/rota |
| 3D transform functions rotateX/rotateY/rotate3d/translateZ/t | CSS Transforms 2 | partial | src/style/computed.rs:6132-6159 approximates rotateX/rotateY as 2D |
| perspective, perspective-origin, transform-style: preserve-3 | CSS Transforms 2 | unsupported | rg found no perspective/backface/transform-style parser or compute |
| position: static and relative offsets | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-static-flow, p |
| position:absolute top/left/right/bottom insets and stretch w | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-absolute-top-l |
| absolute containing block from positioned ancestor | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-absolute-conta |
| position:fixed in paged media, repeated on each printed page | CSS Position 3 | partial | tests/parity/manifest/positioning.json and interactions.json cover |
| position:sticky scroll sticking behavior | CSS Position 3 | na-not-pdf-relevant | Sticky's dynamic scroll response has no live scroll state in stati |
| z-index integer/auto ordering for positioned boxes | CSS Position 3 | supported-tested | tests/parity/manifest/positioning.json: positioning-zindex-higher, |
| stacking context interactions with transform and z-index | CSS Position 3 / CSS Transfo | supported-tested | tests/parity/manifest/interactions.json: positioning-zindex-x-tran |
| absolute/fixed boxes inside flex and grid containers | CSS Position 3 / CSS Display | supported-tested | tests/parity/manifest/interactions.json: positioning-absolute-x-fl |
| overflow: visible and hidden | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-clip |
| overflow: clip | CSS Overflow 3 | supported-untested | tests/parity/manifest/overflow-clipping.json: overflow-clip is mar |
| overflow: scroll and auto as static print clipping | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-scroll-prin |
| overflow-x/overflow-y separate axes and visible/clip compute | CSS Overflow 3 | supported-untested | tests/parity/manifest/overflow-clipping.json: overflow-x-y-separat |
| overflow clipping to padding box | CSS Overflow 3 | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-padding-box |
| overflow clipping combined with border-radius | CSS Overflow 3 / CSS Backgro | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-bord |
| overflow clipping on flex/grid items and nested clipping cha | CSS Overflow 3 / CSS Display | supported-tested | tests/parity/manifest/overflow-clipping.json: overflow-hidden-flex |
| overflow-clip-margin expanding the clip edge | CSS Overflow 3 | unsupported | No overflow-clip-margin handling found under src/ |
| scrollbar-gutter layout reservation for scroll containers | CSS Overflow 3 | unsupported | No scrollbar-gutter handling found under src/ |
| text-overflow: clip and ellipsis on clipped nowrap text | CSS Overflow 3 | supported-tested | tests/parity/manifest/text-advanced.json: text-advanced-text-overf |
| PNG/JPEG replaced image loading and natural dimensions | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-basic-png, img-bas |
| CSS width/height and HTML width/height on replaced images | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-width-height, img- |
| replaced image max-width/max-height clamping | CSS Images 3 / CSS Sizing 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-percent-width, img |
| object-fit: fill/contain/cover/none | CSS Images 3 | supported-untested | tests/parity/manifest/images-replaced.json: img-object-fit-contain |
| object-fit: scale-down | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-fit-scale-d |
| object-position keywords, percentages, and near-edge lengths | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-position-pe |
| object-position far-edge length offsets such as right 20px b | CSS Images 3 | partial | src/style/computed.rs:5368-5398 comments that far-edge length offs |
| replaced content clipped to its content box after object-fit | CSS Images 3 | supported-tested | tests/parity/manifest/images-replaced.json: img-object-fit-cover-p |
| SVG image intrinsic sizing, viewBox, and preserveAspectRatio | CSS Images 3 / SVG 2 | supported-tested | tests/parity/manifest/images-replaced.json: img-svg-as-img-inner-r |
| inline SVG shapes, gradients, clip paths, and text as replac | CSS Images 3 / SVG 2 | partial | tests/parity/manifest/images-replaced.json includes partial SVG in |
| image fragmentation and slicing across pages | CSS Fragmentation / CSS Imag | supported-tested | tests/parity/manifest/images-replaced.json: img-monolithic-page-pu |
| width/height/min-width/max-width/min-height/max-height lengt | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-width-explicit,  |
| aspect-ratio property deriving an auto size | CSS Sizing 4 (shipped) / CSS | supported-untested | tests/parity/manifest/images-replaced.json: img-aspect-ratio-box i |
| box-sizing: content-box and border-box | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-box-sizing-borde |
| intrinsic sizing width:min-content and width:max-content | CSS Sizing 3 | supported-untested | No target parity fixture for min-content/max-content; src/style/co |
| width:fit-content keyword | CSS Sizing 3 | supported-tested | tests/parity/manifest/block-box-model.json: block-width-fit-conten |
| fit-content(<length-percentage>) function | CSS Sizing 3 | partial | src/style/computed.rs:7014-7016 approximates grid fit-content() tr |
| CSS 2.2 block box margins, padding, borders, explicit width/ | CSS 2.2 box model | supported-tested | tests/parity/manifest/block-box-model.json: block-basic, block-pad |
| vertical margin collapsing including sibling, parent/child,  | CSS 2.2 box model | supported-tested | tests/parity/manifest/block-box-model.json: block-margin-collapse- |
| percentage padding and margins resolving against containing  | CSS 2.2 box model / CSS Valu | supported-tested | tests/parity/manifest/units-values.json: units-percent-padding, un |
| absolute length units px, pt, pc, in, cm, mm, Q | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-px-pt, units-pc-mm- |
| font-relative em and rem units | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-em, units-rem, unit |
| font-relative ex and ch units in layout lengths | CSS Values 4 | partial | src/parser/css/values.rs:64-73 preserves Ex/Ch, but src/style/reso |
| viewport units vw, vh, vmin, vmax resolved against the print | CSS Values 4 | supported-untested | tests/parity/manifest/units-values.json: units-viewport-vmin-vmax  |
| small/large/dynamic viewport units svw/svh/lvw/lvh/dvw/dvh a | CSS Values 4 | na-not-pdf-relevant | These distinguish dynamic visual/layout viewport states; static PD |
| calc() arithmetic with mixed units and operator precedence | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-calc-mixed, units-c |
| min(), max(), and clamp() sizing functions | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-min-max, units-clam |
| custom properties var() fallback and nested fallback in leng | CSS Values 4 / CSS Cascade 5 | supported-tested | tests/parity/manifest/units-values.json: units-var-basic, units-va |
| scientific notation numeric values | CSS Values 4 | supported-tested | tests/parity/manifest/units-values.json: units-scientific-numbers; |
| display:block, inline, inline-block, none | CSS Display 3 | supported-tested | tests/parity/manifest/block-box-model.json and interactions.json c |
| display:flex and display:grid as stable layout modes | CSS Display 3 | supported-tested | tests/parity/manifest/interactions.json: flex/grid nesting, flex-w |
| display:contents box suppression while preserving children | CSS Display 3 | unsupported | src/style/computed.rs:25-34 Display enum lacks Contents; parser at |
| display:flow-root establishing a new block formatting contex | CSS Display 3 | unsupported | src/style/computed.rs:25-34 Display enum lacks FlowRoot; parser at |
| CSS Display 3 multi-keyword display syntax such as inline fl | CSS Display 3 | unsupported | src/style/computed.rs:3170-3179 accepts only single keywords none/ |
| type, class, id, universal, attribute selectors and combinat | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: specificity, class/t |
| :nth-child(), :nth-last-child(), :nth-of-type(), first/last/ | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: nth-child odd/even/f |
| :nth-child(An+B of <selector-list>) | Selectors 4 | unsupported | No nth-child 'of selector' parsing found; src/parser/css/selectors |
| :not(), :is(), :where() selector-list matching and specifici | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-no |
| :has() following sibling relative selectors | Selectors 4 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-ha |
| :has() child and descendant relative selectors | Selectors 4 | partial | src/parser/css/selectors.rs:707-746 states child/descendant relati |
| @media print and print media query gating | CSS Conditional 3 | supported-tested | tests/parity/manifest/selectors-cascade.json: selectors-cascade-me |
| @supports property feature queries with and/or/not | CSS Conditional 3 | supported-untested | tests/parity/manifest/selectors-cascade.json: @supports simple cas |
| @supports selector() feature queries | CSS Conditional 3 / Selector | partial | src/parser/css/media.rs:518-519 treats selector(:has(a)) as a leni |
| cascade specificity, source order, inline styles, !important | CSS Cascade 5 | supported-tested | tests/parity/manifest/selectors-cascade.json: specificity, source- |
| cascade layers @layer and revert-layer | CSS Cascade 5 | unsupported | No @layer processing found; src/parser/css/values.rs:8 parses reve |
| dynamic pseudo-classes :hover, :focus, :active and pointer/i | Selectors 4 | na-not-pdf-relevant | Static PDF has no interactive state; src/parser/css/selectors.rs:6 |
| CSS Transforms 2 percentage scale components | CSS Transforms Level 2 | unsupported | The spec grammar allows scale() components as <number-percentage>; |
| CSS Position 3 inset shorthand and logical inset properties | CSS Positioned Layout Level  | unsupported | Spec defines inset, inset-block, inset-inline and longhands; sourc |
| Percentage insets for positioned boxes | CSS Positioned Layout Level  | supported-untested | Computed style stores inset LengthPercent values and resolves perc |
| Negative z-index painting of positioned descendants | CSS Positioned Layout Level  | partial | PDF paint ordering buckets absolutely positioned descendants after |
| overflow-block and overflow-inline logical overflow properti | CSS Overflow Level 3 | unsupported | Spec indexes logical overflow properties; source only parses overf |
| overflow:clip does not establish a new formatting context | CSS Overflow Level 3 | partial | Spec distinguishes clip from hidden: clip clips but does not creat |
| line-clamp and -webkit-line-clamp static clamping | CSS Overflow Level 4 | unsupported | Overflow Level 4 defines line-clamp/max-lines/block-ellipsis; sour |
| text-overflow string values and two-sided overflow markers | CSS Overflow Level 3 and CSS | unsupported | Spec permits string markers and start/end values; existing support |
| image-orientation angle and flip values | CSS Images Level 3 | unsupported | Spec defines image-orientation: from-image / none / <angle> // fli |
| image-rendering interpolation keywords | CSS Images Level 3 | unsupported | Spec defines image-rendering keywords such as pixelated and crisp- |
| Aspect-ratio deriving inline size from definite block size | CSS Sizing Level 4 | partial | Round 1 covered width-derived height. Source helpers emphasize hei |
| CSS Values 4 stepped and advanced math functions | CSS Values and Units Level 4 | unsupported | Spec defines round(), mod(), rem(), sin(), cos(), tan(), hypot(),  |
| CSS Cascade all shorthand reset | CSS Cascade Level 5 | unsupported | The all shorthand is cascade-relevant and resets nearly all proper |
| CSS Display list-item and table display values on arbitrary  | CSS Display Level 3 | unsupported | Spec includes list-item and table internal display values; the com |
| Selectors 4 static language, direction, and defined pseudo-c | Selectors Level 4 | unsupported | Round 1 omitted :lang(), :dir(), and :defined; selector source imp |
| Selectors 4 nth-last-of-type structural selector | Selectors Level 4 | supported-untested | Selector context tracks following siblings, but round-1 matrix lis |
| min-width and max-width intrinsic sizing keywords | CSS Sizing Level 3 | partial | Round 1 covered width:min-content/max-content, but min-width/max-w |
| Selectors 4 :scope in document stylesheets | Selectors Level 4 | supported-untested | Source maps :scope without an explicit scoping root to the documen |
| CSS Transforms 2 translate3d, scale3d, scaleZ, and 3D transf | CSS Transforms Level 2 | unsupported | Round 1 grouped 3D transforms broadly; source accepts rotateX/rota |
| CSS Values 4 lh, rlh, and cap font-relative units | CSS Values and Units Level 4 | unsupported | Spec defines line-height and cap-height units; parser supports em/ |
| CSS Values 4 small/large/dynamic viewport units | CSS Values and Units Level 4 | unsupported | Spec defines svw/svh/lvw/lvh/dvw/dvh viewport variants; parser sup |
| Selectors 4 :has adjacent and general sibling relative selec | Selectors Level 4 | supported-untested | Source has explicit :has(+ ...) and :has(~ ...) branches, while ro |
