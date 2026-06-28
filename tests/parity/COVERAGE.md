# ironpress — CSS/HTML feature & parity coverage tracker

> Living master doc: every PDF-relevant CSS/HTML feature, ironpress support + test status, and every verified coverage gap. Built from an exhaustive adversarial spec-audit (finders fetch live W3C/WHATWG/MDN specs; every verdict comes from an actual ironpress-vs-oracle 300dpi render). Auto-generated from tests/parity/report.json.

## Status

- **Corpus:** 1239 fixtures — 692 implemented (locked regression tests), 510 tracked-unsupported.
- **Parity gate:** 787P / 447F / 5U · scored 63.78% (common-set). Tracked-unsupported FAILs are non-gating; each flips to PASS as its engine fix lands.
- **Real-bug backlog:** 446 verified ironpress defects (tracked-unsupported + currently FAIL), each with a spec-correct oracle ref.
- **Suspects under review:** 59 unsupported-but-PASS fixtures being reclassified (retag-implemented / fix-or-drop).

### Real-bug backlog by diagnosis class

| class | n | meaning |
|---|--:|---|
| ColorValue | 174 | wrong color (recolour / gradient stop / filter math / blend) |
| Missing | 133 | feature not painted (blank where oracle paints) |
| Extra | 117 | ironpress paints where oracle does not |
| GeometryShift | 16 | right pixels, wrong position |
| AaOnly | 4 | sub-pixel antialias only |
| GeometrySize | 2 | wrong size |

### Real-bug backlog by fix-effort (wave ordering)

| effort | n |
|---|--:|
| easy-parse | 57 |
| moderate | 210 |
| hard-feature | 179 |

## Real-bug backlog by subsystem (category)

| category | bugs | median diff% | dominant class | sample defects |
|---|--:|--:|---|---|
| grid | 53 | 61 | Missing | absolute-positioning; align-content; align-items; direction |
| tables | 42 | 57 | ColorValue | anonymous-table-fixup; background-layering; border-collapse; caption |
| flexbox | 38 | 37 | ColorValue | abspos-flex-child; align-content; align-items; align-self |
| clip-mask | 34 | 51 | Extra | -webkit-mask-position; clip; clip-path: circle(); clip-path: circle(farthest-side at ...) |
| paged-media | 34 | 24 | Missing | break-after; break-before; break-precedence; flex-fragmentation |
| filters | 32 | 23 | ColorValue | backdrop-filter; color-interpolation-filters; filter compositing order with clip-path; filter compositing order with mask-image |
| effects | 29 | 55 | ColorValue | background-blend-mode; background-blend-mode comma-separated list; box-shadow inset blur with border-radius; box-shadow positive spread with blur and border-radius |
| backgrounds-borders | 26 | 34 | ColorValue | background-attachment; background-blend-mode; background-clip; background-image |
| text-advanced | 20 | 7 | ColorValue | dir=auto; direction:rtl basic inline paragraph; hyphens; line-break:anywhere |
| backgrounds-gradients | 16 | 83 | ColorValue | background-origin; conic-gradient; linear-gradient; multiple-backgrounds |
| generated-content | 14 | 15 | Missing | ::before{display:list-item}; ::first-letter numeric first-letter unit; ::first-letter punctuation unit; ::first-line letter-spacing |
| lists-counters | 13 | 79 | Missing | ::marker font-size; ::marker{content:...}; <li value>; <ol reversed> |
| fonts-advanced | 12 | 13 | Missing | @font-face font-style descriptor; @font-face font-weight descriptor; @font-face size-adjust descriptor; @font-face src:local() |
| multicol | 12 | 47 | Extra | break-after; break-before; break-inside; column-fill |
| color-opacity | 11 | 100 | ColorValue | background-color; color(display-p3 ...); color(srgb ...); color(srgb-linear ...) |
| inline-text | 10 | 10 | Missing | text-decoration-line:underline overline; text-decoration-style:wavy; text-decoration-thickness:6px; text-indent |
| transforms | 10 | 35 | ColorValue | individual-transform; transform-3d |
| overflow-clipping | 8 | 28 | ColorValue | line-clamp; overflow; overflow-clip-margin; scrollbar-gutter |
| selectors-cascade | 8 | 100 | ColorValue | cascade; cascade-layers; pseudo-class; selectors-4 |
| block-box-model | 6 | 52 | Missing | display; max-width; min-width |
| positioning | 5 | 18 | ColorValue | inset; position; z-index |
| typography | 5 | 39 | Missing | initial-letter:2; line-height; line-height:<length em> inheritance; mixed inline font-size line box metrics |
| units-values | 5 | 21 | Extra | font-relative-length; viewport-units |
| images-replaced | 2 | 69 | ColorValue | inline-svg; object-position |
| interactions | 1 | 26 | Missing | supports |

## Full real-bug inventory (per subsystem)

### grid (53)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| grid-row-spanning-grid-item-fragments-across-spanned-rows | fragmentation/spanned-grid-area | 100 | AaOnly | A grid item spanning multiple rows should keep its area geometry and fragment content cons |
| grid-tall-grid-item-fragments-inside-its-grid-area | fragmentation/grid-item | 100 | Missing | A tall block child inside one grid item should fragment across pages without escaping the  |
| grid-auto-rows-repeats-a-track-size-pattern | grid-auto-rows/pattern | 98 | Missing | Implicit rows should cycle through the grid-auto-rows list: 30px, 70px, 30px, ; catches st |
| grid-template-shorthand | grid-template/areas-and-tracks | 98 | Missing | The grid-template shorthand defines area strings plus row and column tracks; catches engin |
| grid-subgrid-adopts-parent-row-tracks | subgrid/rows | 95 | Missing | A nested grid spanning two parent rows should use the parent row sizes for its own childre |
| grid-align-content-positions-grid-rows-in-a-tall-container | align-content/end | 95 | Extra | align-content:end should pack fixed-height rows at the block-end of a taller grid containe |
| grid-invalid-template-areas | grid-template-areas/non-rectangula | 95 | Extra | A non-rectangular named area invalidates grid-template-areas, so named placement falls bac |
| grid-subgrid-columns | subgrid/columns | 95 | Missing | grid-template-columns:subgrid makes a nested grid adopt parent column tracks; catches inde |
| grid-subgrid-augments-and-exposes-line-names | subgrid/line-name-augmentation | 93 | Missing | Line names declared after subgrid should be usable by grandchildren placed inside the subg |
| grid-display-contents-flattens-children-into-grid | display/contents-grid-items | 92 | ColorValue | A display:contents wrapper should not consume a grid cell; its two children should become  |
| grid-repeated-named-lines-resolve-nth-occurrence | grid-column/repeated-named-lines | 92 | ColorValue | grid-column: 2 col / 3 col should select later occurrences of the repeated line name; catc |
| grid-container-pseudo-elements-are-grid-items | generated-content/pseudo-grid-item | 91 | ColorValue | A grid container's ::before generated box should participate as the first grid item before |
| grid-vertical-writing-mode-changes-grid-physical-axes | writing-mode/vertical-rl-grid | 83 | ColorValue | In vertical-rl writing mode, the inline axis is vertical, so grid rows and columns map to  |
| grid-subgrid-inherits-parent-gutters-on-subgridded-axis | subgrid/inherited-gutters | 77 | Missing | Children of a column subgrid should align to parent columns including the parent's column- |
| grid-rows-fragment-between-pages | fragmentation/grid-rows | 76 | Missing | A grid with fixed-height rows taller than the page should break between rows rather than c |
| grid-shorthand-defines-auto-flow-rows | grid/auto-flow-shorthand | 75 | Missing | The grid shorthand should set explicit columns and grid-auto-flow/auto rows in one declara |
| grid-auto-rows-use-wrapped-content-height | grid-template-rows/auto-intrinsic | 73 | Missing | An auto row containing wrapping text should grow to the full multi-line content height bef |
| grid-order-modified-placement-also-applies-to-dense-packing | order/dense-grid-auto-placement | 71 | ColorValue | Dense auto-placement must process items in order-modified document order before deciding w |
| grid-inline-grid-participates-in-inline-flow | display/inline-grid | 71 | Missing | An inline-grid between two inline labels should remain on the same text line and occupy on |
| grid-unequal-grid-template-areas-row-lengths-invalidate-declaration | grid-template-areas/unequal-row-in | 71 | Extra | Rows with different numbers of cell tokens make grid-template-areas invalid, so grid-area  |
| grid-shorthand-resets-omitted-implicit-grid-properties | grid/shorthand-reset | 70 | ColorValue | A later grid shorthand should reset a previous grid-auto-flow:column to row flow when omit |
| grid-column-dense-auto-placement-backfills-holes | grid-auto-flow/column-dense | 69 | Extra | In column flow, dense packing should backfill a later one-row item into an earlier hole in |
| grid-justify-content-centers-the-grid-tracks | justify-content/center | 67 | ColorValue | A grid narrower than its container should be centered along the inline axis by justify-con |
| grid-placement-longhands | grid-placement/longhands | 63 | Missing | grid-column-start/end and grid-row-start/end longhands place a two-column two-row item; ca |
| grid-rtl-direction-reverses-grid-column-line-start | direction/rtl-grid-line-numbering | 62 | Missing | With direction:rtl, grid-column:1 / 2 should address the rightmost column line interval; c |
| grid-span-named-line-searches-to-matching-line | grid-column/span-named-line | 61 | ColorValue | grid-column: left / span right should span from the left named line to the next right name |
| grid-span-integer-named-line-syntax-is-honored | grid-column/span-integer-named-lin | 61 | ColorValue | grid-column: span 2 target / target 3 should count named lines as well as a numeric span;  |
| grid-direct-text-creates-anonymous-grid-items | grid-items/anonymous-text | 60 | Extra | Direct text between element children in a grid container should occupy its own anonymous g |
| grid-legacy-grid-row-and-column-gap-aliases-apply-separately | gap/legacy-grid-gap-aliases | 58 | Extra | grid-row-gap and grid-column-gap should map to row-gap and column-gap with independent val |
| grid-auto-margins-align-grid-items-before-self-alignment | grid-items/auto-margins | 57 | Extra | A grid item with margin-left:auto should absorb free inline space and move to the right ed |
| grid-order-auto-placement | order/grid-auto-placement | 56 | ColorValue | Grid auto-placement follows order-modified document order, yielding yellow-green-red inste |
| grid-auto-columns-repeats-a-track-size-pattern | grid-auto-columns/pattern | 55 | Missing | Implicit columns created by explicit placement should cycle through grid-auto-columns valu |
| grid-overlap-z-index | z-index/static-grid-items | 54 | ColorValue | Overlapping static grid items honor z-index, so an earlier higher-z-index inset item paint |
| grid-items-baseline-align-across-a-row | align-items/baseline | 53 | Extra | align-items:baseline should align text baselines of differently sized grid items rather th |
| grid-intrinsic-track-keywords | grid-template-columns/min-content- | 48 | ColorValue | min-content and max-content columns size the same breakable text differently; catches engi |
| grid-template-areas-create-implicit-tracks-when-track-lists-are-omitted | grid-template-areas/implicit-track | 47 | ColorValue | grid-template-areas alone should define a two-column/two-row explicit grid whose unsized t |
| grid-justify-content-distributes-free-space-between-tracks | justify-content/space-between | 47 | Extra | space-between should put the first track at the left edge and the second at the right edge |
| grid-item-margins-inset-the-item-box | grid-items/margins | 46 | Extra | A grid item with margins should paint inset from its grid area and leave white space aroun |
| grid-repeat-auto-fit-collapse | grid-template-columns/repeat-auto- | 42 | ColorValue | repeat(auto-fit, minmax(80px, 1fr)) collapses the unused repeated track and stretches four |
| grid-fit-content-track | grid-template-columns/fit-content | 38 | ColorValue | fit-content(120px) clamps the first track while a 1fr track takes the remainder; catches i |
| grid-auto-columns-implicit | grid-auto-columns/implicit-columns | 38 | Missing | An item placed in column 3 creates implicit columns sized by grid-auto-columns; catches en |
| grid-repeat-auto-fill-count | grid-template-columns/repeat-auto- | 32 | Extra | repeat(auto-fill, 80px) resolves the repetition count from a 500px grid and keeps five cel |
| grid-auto-placement-handles-definite-row-only | grid-auto-placement/definite-row | 32 | Missing | Items pinned to row 2 but with auto columns should fill row 2 columns in order without con |
| grid-minmax-auto-minimum-honors-content | grid-template-columns/minmax-auto- | 30 | Missing | A minmax(auto,1fr) column with a long unbreakable word should not shrink below the word's  |
| grid-place-content-sets-block-and-inline-grid-distribution | place-content/end-center | 29 | Extra | place-content:end center should set align-content:end and justify-content:center for the g |
| grid-auto-placement-handles-definite-column-only | grid-auto-placement/definite-colum | 24 | Missing | Items pinned to column 3 but with auto rows should stack down column 3 without occupying e |
| grid-percentage-rows-resolve-against-grid-height | grid-template-rows/percentage | 24 | ColorValue | Percentage row tracks in a definite-height grid should resolve from the grid content-box h |
| grid-negative-grid-line-numbers-count-from-the-end | grid-column/negative-lines | 23 | Extra | grid-column: -3 / -1 should span the last two columns of a four-column explicit grid; catc |
| grid-item-percentage-margins-use-grid-area-inline-size | grid-items/percentage-margins | 18 | Extra | A 10% left/right margin on a 200px grid area should create 20px side insets, independent o |
| grid-percentage-grid-gaps-resolve-from-container-size | gap/percentage | 18 | Missing | A 10% column gap in a 200px grid should create a 20px gutter between two 90px columns; cat |
| grid-absolute-grid-child-positions-against-its-specified-grid-area | absolute-positioning/grid-area-con | 15 | ColorValue | An abspos child with grid-column:2 / 3 and grid-row:2 / 3 should use that grid area as its |
| grid-spanning-item-contributes-to-auto-columns | grid-template-columns/auto-spannin | 15 | Extra | A wide item spanning two auto columns should contribute to both tracks instead of leaving  |
| grid-minmax-intrinsic-endpoints-are-distinct | grid-template-columns/minmax-intri | 12 | ColorValue | A minmax(min-content,max-content) track should size from intrinsic text constraints, not a |

### tables (42)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| tables-anonymous-table-cell-siblings | anonymous-table-fixup/orphan-table | 100 | Missing | Two inline siblings are display:table-cell without an explicit table or row; fixup creates |
| tables-caption-min-width | caption/caption-min-width-auto-lay | 100 | Missing | A shrink-to-fit auto table has tiny cells but a wide nowrap caption that should expand the |
| tables-css-row-groups-reorder | display/css-row-group-reorder | 100 | Missing | A div-based CSS table declares footer before body and header last; table display semantics |
| tables-empty-cells-collapse-ignored | empty-cells/ignored-in-collapse | 100 | Missing | A collapsed-border table sets empty-cells:hide on an empty red cell next to a filled green |
| tables-break-before-row | table-fragmentation/break-before-r | 100 | Extra | The second table row declares break-before:page on a short page and should begin page two. |
| tables-visibility-collapse-colgroup | visibility-collapse/column-group | 95 | Extra | A two-column colgroup is collapsed, leaving only the trailing green column visible. Catche |
| tables-caption-side-on-caption | caption/caption-side-on-caption | 91 | ColorValue | The table has no caption-side declaration, but the caption element itself sets caption-sid |
| tables-column-background | background-layering/column-backgro | 86 | ColorValue | Two col elements provide red and green backgrounds while all cells are transparent. Catche |
| tables-direction-rtl-columns | direction/rtl-column-order | 85 | ColorValue | A direction:rtl table has red first cell and green second cell; the first column should be |
| tables-row-group-background | background-layering/row-group-back | 84 | ColorValue | A tbody has a blue background, cells are transparent, and the table background is yellow.  |
| tables-cellspacing-attribute | html-table-attributes/cellspacing | 83 | Missing | A legacy table uses cellspacing=14 with no CSS border-spacing and a dark table background. |
| tables-row-height-minimum | row-height/row-min-height | 83 | Missing | A tr declares height:80px while its cells contain only a 20px marker. Catches wrong implem |
| tables-css-table-columns-fixed | display/css-table-column-widths | 83 | Extra | A CSS display table uses non-col elements as table-column boxes to make a narrow red colum |
| tables-table-height-minimum | table-height/table-min-height | 76 | Missing | A table declares height:120px but contains a single short row and has a visible table back |
| tables-inline-table-inline-flow | display/inline-table-inline-flow | 75 | Missing | Text before and after an inline-table should remain on the same line while the inline-tabl |
| tables-visibility-collapse-row-group | visibility-collapse/row-group | 73 | Extra | The tbody is visibility:collapse and should remove all body rows while leaving the footer  |
| tables-css-display-table | display/css-table-roles | 73 | Extra | display:table/table-row/table-cell on divs lays out side-by-side cells with separated spac |
| tables-rowspan-zero | rowspan/rowspan-zero | 64 | Missing | rowspan=0 spans the remaining rows in the row group; catches implementations that clamp ze |
| tables-visibility-collapse-column | visibility-collapse/column | 61 | Extra | The middle col is visibility:collapse in a three-column fixed table. Catches wrong impleme |
| tables-rowspan-thead-clipped | rowspan/row-group-boundary-clip | 58 | ColorValue | A thead cell has rowspan=3 but the thead has one row; it must not occupy the first tbody c |
| tables-collapse-outer-table-border | border-collapse/outer-table-border | 57 | ColorValue | The table has a thick red collapsed border while the outer cells have thinner blue borders |
| tables-colgroup-background | background-layering/column-group-b | 56 | ColorValue | A colgroup with span=2 has a blue background under two transparent columns and a third col |
| tables-auto-colspan-distribution | table-layout/auto-colspan-distribu | 51 | ColorValue | Auto table layout distributes a colspan width contribution over unequal existing column ba |
| tables-visibility-collapse-cell | visibility-collapse/cell | 50 | ColorValue | One cell in a two-cell row is visibility:collapse; its grid slot remains but its red paint |
| tables-break-inside-avoid-tbody | table-fragmentation/tbody-break-in | 49 | Extra | A tbody with two rows has break-inside:avoid and fits on a fresh page but not in the remai |
| tables-anonymous-cell-in-row | anonymous-table-fixup/non-cell-row | 49 | Missing | A CSS table-row contains one explicit table-cell and one plain div; the div should be wrap |
| tables-border-attribute | html-table-attributes/border | 41 | Missing | A table uses border=4 without CSS borders, so browser presentational hints draw visible ta |
| tables-second-header-group-not-repeat | table-fragmentation/second-header- | 37 | Missing | A CSS display table owns two table-header-group boxes; only the first is a header, the sec |
| tables-multiple-captions | caption/multiple-caption-boxes | 34 | Extra | Two CSS table-caption boxes on the same CSS table use distinct colors and should both part |
| tables-css-table-caption | display/css-table-caption | 33 | Extra | A non-caption div inside a CSS table is made display:table-caption and should become a ful |
| tables-empty-cells-hidden-content | empty-cells/visibility-hidden | 32 | ColorValue | empty-cells:hide treats visibility:hidden cell content as no visible content and removes a |
| tables-rowspan-height-distribution | rowspan/height-distribution | 24 | Extra | A cell with rowspan=2 and height:100px spans two otherwise short rows. Catches wrong imple |
| tables-visibility-collapse-row | visibility-collapse/row | 17 | ColorValue | The middle row is visibility:collapse in a separated table with vertical border-spacing. C |
| tables-rowspan-extra-columns | rowspan/extra-columns-from-occupan | 16 | Missing | The first row has a rowspan in column one plus one normal cell; the second row has two new |
| tables-cellpadding-attribute | html-table-attributes/cellpadding | 12 | ColorValue | A legacy table uses cellpadding=16 with no CSS padding; an inner marker should be inset fr |
| tables-border-hidden-conflict | border-collapse/hidden-conflict | 7 | ColorValue | Collapsed-border conflict where a hidden right border must suppress the neighboring solid  |
| tables-border-style-conflict | border-collapse/style-conflict | 7 | ColorValue | Collapsed-border conflict where equal-width double beats solid on the shared edge; catches |
| tables-fixed-overflow-clip | table-layout/fixed-overflow-clip | 6 | Extra | table-layout:fixed clips nowrap overflow:hidden text inside its fixed-width cell; catches  |
| tables-border-width-conflict | border-collapse/width-conflict | 6 | ColorValue | Collapsed-border conflict where the wider blue shared border wins over a narrower red neig |
| tables-collapse-directional-tie | border-collapse/directional-tie-br | 5 | ColorValue | Equal-width equal-style borders of different colors meet at a collapsed shared vertical ed |
| tables-colspan-max-clamp | html-table-attributes/colspan-max- | 5 | ColorValue | A cell declares colspan=1001 before a normal cell; browsers clamp overly large colspan val |
| tables-vertical-align-text-bottom | vertical-align/text-bottom-fallbac | 2 | ColorValue | vertical-align:text-bottom does not apply to table cells and should use baseline behavior; |

### flexbox (38)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| flexbox-display-inline-flex | display/inline-flex | 100 | Missing | display:inline-flex creates an inline-level flex container that shares the line with surro |
| flexbox-break-after-item | fragmentation/break-after-item | 100 | Extra | break-after:page on a column flex item creates a forced page break after that item; catche |
| flexbox-break-before-item | fragmentation/break-before-item | 100 | Extra | break-before:page on a column flex item creates a forced page break before that item; catc |
| flexbox-fragmentation-wrap-pages | fragmentation/wrapped-lines-across | 100 | GeometryShift | A wrapped row flex container whose lines exceed one page fragments between flex lines; cat |
| flexbox-direction-rtl-row | flex-direction/row-rtl-main-start | 88 | ColorValue | flex-direction:row follows direction:rtl so source order starts at the right main-start ed |
| flexbox-flex-shorthand-basis-only | flex-shorthand/basis-only | 86 | Missing | The one-token flex:80px shorthand is a flex-basis form with grow and shrink enabled; catch |
| flexbox-row-reverse-rtl | flex-direction/row-reverse-rtl | 76 | ColorValue | direction:rtl plus flex-direction:row-reverse reverses the RTL inline axis so visual order |
| flexbox-column-flex-shrink | flex-shrink/column | 75 | ColorValue | In a definite-height column flex container, negative free space is removed using scaled sh |
| flexbox-aspect-ratio-height-transfer | aspect-ratio/height-to-basis | 68 | Missing | A definite height plus aspect-ratio transfers into an auto main-size flex base in a row; c |
| flexbox-flex-basis-min-content | flex-basis/min-content | 59 | Missing | flex-basis:min-content uses the min-content contribution as the flex base size; catches pa |
| flexbox-writing-mode-vertical-rl-column | writing-mode/vertical-rl-column | 56 | Missing | In writing-mode:vertical-rl, flex-direction:column follows the horizontal block axis right |
| flexbox-flex-basis-fit-content | flex-basis/fit-content | 55 | Missing | flex-basis:fit-content shrink-to-fit sizes the flex base rather than using a declared widt |
| flexbox-display-inline-flex-two-keyword | display/inline-flex-two-keyword | 49 | ColorValue | display:inline flex creates an inline-level flex container in a text line; catches parsers |
| flexbox-aspect-ratio-grow-cross-size | aspect-ratio/cross-size-after-grow | 46 | ColorValue | Zero-basis growing flex items with aspect-ratio derive auto cross size from their final gr |
| flexbox-order-decimal-invalid | order/integer-grammar | 43 | ColorValue | order:1.5 is invalid because order accepts integers, so source order is preserved; catches |
| flexbox-align-content-safe-center | align-content/safe-center | 42 | ColorValue | align-content:safe center centers wrapped flex lines when they fit; catches parsers that r |
| flexbox-writing-mode-vertical-rl-row | writing-mode/vertical-rl-row | 40 | ColorValue | In writing-mode:vertical-rl, flex-direction:row follows the vertical inline axis and stack |
| flexbox-place-items-center | place-items/align-items | 39 | Extra | place-items:center sets align-items:center for a flex row container; catches grid-only pla |
| flexbox-container-width-max-content | intrinsic-sizing/max-content-width | 38 | Extra | width:max-content shrink-wraps a flex container to its flex line rather than filling the c |
| flexbox-column-min-height-auto | automatic-minimum-size/column-min- | 36 | Extra | In flex-direction:column, min-height:auto gives a shrinkable item a content-based main-axi |
| flexbox-flex-basis-calc-percent | flex-basis/calc-percent | 36 | ColorValue | flex-basis:calc(25% - 10px) resolves against the definite row main size; catches implement |
| flexbox-writing-mode-vertical-lr-row | writing-mode/vertical-lr-row | 34 | Missing | writing-mode:vertical-lr with flex-direction:row follows the vertical inline axis; catches |
| flexbox-place-content-justify | place-content/justify-content | 33 | Extra | place-content:center space-between sets flex justify-content:space-between in a row contai |
| flexbox-display-contents-items | display/contents-flex-items | 32 | Missing | A display:contents child of a flex container is box-suppressed so its element children bec |
| flexbox-container-width-min-content | intrinsic-sizing/min-content-width | 30 | Extra | width:min-content gives a flex container an intrinsic width rather than a page-wide block  |
| flexbox-anonymous-text-items | flex-items/anonymous-text | 30 | ColorValue | Direct text around an element child becomes anonymous flex items and remains visible in th |
| flexbox-flex-basis-max-content | flex-basis/max-content | 27 | ColorValue | flex-basis:max-content must use the item's unwrapped max-content contribution instead of i |
| flexbox-align-items-last-baseline | align-items/last-baseline | 26 | ColorValue | align-items:last baseline aligns a one-line item's baseline with the last baseline of a tw |
| flexbox-place-self-end | place-self/align-self | 24 | Extra | place-self:end sets flex align-self to the cross-end side; catches grid-only place-self im |
| flexbox-justify-safe-center-overflow | justify-content/safe-center-overfl | 24 | Extra | justify-content:safe center falls back to start alignment when non-shrinking items overflo |
| flexbox-align-items-safe-center | align-items/safe-center | 24 | Extra | align-items:safe center is equivalent to center when the item fits; catches parsers that r |
| flexbox-abspos-static-position | abspos-flex-child/static-position- | 22 | ColorValue | An absolutely positioned flex child with auto insets uses the flex static-position rectang |
| flexbox-align-self-safe-center | align-self/safe-center | 20 | Extra | align-self:safe center overrides container align-items:flex-start when the item fits; catc |
| flexbox-flex-shorthand-grow-basis | flex-shorthand/grow-basis | 15 | ColorValue | The two-token flex:1 80px shorthand means grow factor plus basis, not grow plus shrink; ca |
| flexbox-overflow-wrap-anywhere-auto-min | automatic-minimum-size/overflow-wr | 14 | Extra | overflow-wrap:anywhere lowers a long-word flex item's automatic minimum size so the row ca |
| flexbox-overflow-hidden-auto-min | automatic-minimum-size/overflow-hi | 11 | Extra | overflow:hidden alone makes a shrinking row flex item's automatic minimum size zero; catch |
| flexbox-baseline-empty-synthesis | align-items/baseline-empty-item | 11 | ColorValue | align-items:baseline synthesizes a baseline for an empty fixed-size item rather than falli |
| flexbox-min-width-zero-auto-min | automatic-minimum-size/min-width-z | 10 | Extra | min-width:0 plus overflow:hidden disables the flex item's automatic min-content floor so e |

### clip-mask (34)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| r2-mask-origin-padding-box | mask-origin/mask-origin-padding-bo | 96 | Extra | mask-origin must use the padding box as positioning area: A single solid box uses a CSS ma |
| r2-webkit-mask-position-offset | -webkit-mask-position/webkit-mask- | 91 | Extra | -webkit-mask-position alias must behave like mask-position: Only the prefixed mask-positio |
| r2-mask-position-offset-layer | mask-position/mask-position-offset | 91 | Extra | mask-position must offset mask images: A single solid box uses a CSS mask longhand or grad |
| clip-path-inset-content-box | clip-path: inset()/content-box-geo | 89 | Extra | clip-path: inset(0) content-box clips a bordered, padded box to its content-box reference; |
| r2-mask-shorthand-position-size-repeat-alpha | mask shorthand full grammar/shorth | 88 | Extra | mask shorthand must expand position/size/repeat/mode/composite: The mask shorthand sets a  |
| r2-mask-size-independent-layer | mask-size/mask-size-layer | 86 | Extra | mask-size must size the mask layer independently: A single solid box uses a CSS mask longh |
| r2-mask-clip-content-box | mask-clip/mask-clip-content-box | 78 | Extra | mask-clip must limit the mask painting area: A single solid box uses a CSS mask longhand o |
| r2-clip-path-content-box-geometry | clip-path: content-box/content-box | 76 | Extra | standalone clip-path geometry boxes must clip to that box: A single colored box isolates t |
| r2-mask-composite-subtract | mask-composite: subtract/composite | 75 | Extra | mask-composite:subtract must combine multiple mask layers: Two mask layers, a circle and a |
| r2-clip-path-path-triangle | clip-path: path()/path-triangle | 68 | Extra | clip-path path() basic shape must render: A single colored box isolates the clip geometry  |
| r2-mask-composite-add | mask-composite: add/composite-add | 67 | Extra | mask-composite:add must combine multiple mask layers: Two mask layers, a circle and a rect |
| r2-clip-path-ellipse-position-keywords | clip-path: ellipse() with right/bo | 63 | Extra | ellipse() position keywords must resolve: A single colored box isolates the clip geometry  |
| r2-mask-composite-intersect | mask-composite: intersect/composit | 60 | Extra | mask-composite:intersect must combine multiple mask layers: Two mask layers, a circle and  |
| r2-clip-path-rect-rounded | clip-path: rect()/rect-rounded | 59 | Extra | clip-path rect() basic shape must render: A single colored box isolates the clip geometry  |
| r2-clip-path-xywh-rounded-rect | clip-path: xywh()/xywh-rounded-rec | 59 | Extra | clip-path xywh() basic shape must render: A single colored box isolates the clip geometry  |
| r2-clip-property-rect-absolute | clip/deprecated-clip-rect | 55 | Extra | Deprecated clip:rect() still clips absolutely positioned elements: An absolutely positione |
| r2-clip-path-circle-default-closest-side | clip-path: circle()/circle-default | 52 | Extra | circle() default radius must resolve closest-side: A single colored box isolates the clip  |
| r2-clip-path-ellipse-extent-keywords | clip-path: ellipse(closest-side fa | 50 | Extra | ellipse() extent keywords must parse: A single colored box isolates the clip geometry with |
| r2-mask-inline-svg-default-luminance | mask-image: url(#mask) default mas | 50 | Extra | Inline SVG <mask> default luminance must hide black regions: A CSS box references an inlin |
| r2-mask-image-repeating-conic-spokes | mask-image: repeating-conic-gradie | 47 | Missing | repeating-conic-gradient masks must repeat angular bands: A single solid box uses a CSS ma |
| r2-clip-path-url-inline-svg-circle | clip-path: url(#clip)/url-inline-s | 44 | Extra | clip-path url(#clipPath) must reference inline SVG clip sources: A CSS box references an i |
| r2-mask-repeat-tiled-stripes | mask-repeat/mask-repeat-stripes | 44 | Extra | mask-repeat must tile the mask layer: A single solid box uses a CSS mask longhand or gradi |
| r2-mask-image-repeating-radial-rings | mask-image: repeating-radial-gradi | 43 | Extra | repeating-radial-gradient masks need direct coverage: A single solid box uses a CSS mask l |
| r2-mask-border-gradient-ring | mask-border-source/slice/width/rep | 42 | Extra | mask-border must slice and apply a border-box mask: A mask-border gradient should reveal o |
| r2-mask-svg-image-match-source-luminance | mask-image: url(data:image/svg+xml | 33 | ColorValue | SVG image masks in match-source mode use luminance: An SVG mask image has a red half and a |
| r2-mask-svg-image-alpha-mode | mask-mode: alpha with SVG image ma | 33 | ColorValue | mask-mode:alpha on SVG images must ignore luminance: The same red/white SVG mask is used w |
| r2-mask-mode-match-source-gradient-alpha | mask-mode: match-source with CSS g | 33 | Extra | mask-mode:match-source on CSS gradients uses alpha, not luminance: A single solid box uses |
| mask-composite-exclude-layers | mask-composite/exclude-two-layers | 23 | Extra | Two mask-image layers with mask-composite:exclude punch a circular hole through a full-box |
| r2-mask-inline-svg-alpha-type | mask-image: url(#mask) with mask-t | 22 | Extra | Inline SVG <mask> references must honor mask-type:alpha: A CSS box references an inline SV |
| r2-clip-path-ellipse-default-radii | clip-path: ellipse()/ellipse-defau | 21 | Extra | ellipse() default radii must resolve: A single colored box isolates the clip geometry with |
| clip-path-polygon-evenodd | clip-path: polygon()/evenodd-fill- | 17 | Extra | clip-path: polygon(evenodd, ...) cuts an inner rectangular hole; catches polygon clipping  |
| r2-clip-path-url-clip-rule-evenodd | clip-rule with clip-path/url-clip- | 17 | Extra | clip-rule evenodd must cut holes in referenced SVG clipPath: An inline clipPath contains t |
| r2-clip-path-circle-farthest-side | clip-path: circle(farthest-side at | 4 | Extra | circle() keyword radii must parse: A single colored box isolates the clip geometry without |
| r2-clip-path-inset-round-elliptical-radii | clip-path: inset() round elliptica | 2 | Extra | clip-path inset round must support elliptical and per-corner radii: A single colored box i |

### paged-media (34)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| paged-break-precedence-before-left-wins | break-precedence/break-before-left | 100 | GeometryShift | At a shared class-A break point, break-before:left on the following block must override br |
| paged-break-precedence-before-wins | break-precedence/break-before-over | 100 | AaOnly | At a shared class-A break point, break-before:right on the following block must win over b |
| paged-footnote-max-height | footnote/area-max-height | 100 | Extra | @footnote max-height constrains note placement; catches footnote extractors that ignore @f |
| paged-named-page-continuation | named-page/continuation-geometry | 100 | ColorValue | A tall element assigned page:chapter keeps the chapter page size and margin on continuatio |
| paged-named-page-left-right | page-selector/named-left-right | 100 | AaOnly | Named page selectors combined with :left and :right apply spread-specific margins only to  |
| paged-bordered-tall-image-fragment | replaced-fragmentation/bordered-ta | 100 | Missing | A bordered tall replaced image must continue its image content and border decoration acros |
| paged-tall-image-object-fit-fill-fragment | replaced-fragmentation/tall-object | 100 | AaOnly | A tall object-fit:fill image must show continued colored bands across later pages; catches |
| paged-flex-column-fragmentation | flex-fragmentation/column-children | 98 | ColorValue | A vertical flex container taller than one page should continue its child items on later pa |
| paged-grid-fragmentation-rows | grid-fragmentation/rows-across-pag | 98 | Missing | Rows in a one-column grid continue across page fragments instead of clipping as one atomic |
| paged-break-after-avoid-keep-next | break-after/avoid-keep-with-next | 83 | Extra | break-after:avoid keeps a bottom-of-page heading with the following paragraph; catches imp |
| paged-break-before-avoid-page-keep-prev | break-before/avoid-page-keep-with- | 83 | Extra | break-before:avoid-page keeps a heading and following paragraph together across a page bou |
| paged-first-right-cascade | page-selector/first-plus-right-cas | 66 | Extra | The first page also matches :right, so declarations from @page :first and @page :right bot |
| paged-page-margin-three-values | page-margin/three-value-shorthand | 63 | Missing | @page margin with three values resolves top, horizontal, and bottom margins; catches parse |
| paged-page-margin-percentage | page-margin/percentage | 60 | Missing | @page margin:10% creates a visible inset from the page box; catches page margin parsers th |
| paged-blank-margin-box-specificity | page-margin-box/blank-over-right-s | 42 | Missing | On a blank page inserted by break-after:left, @page :blank top-center content overrides @p |
| paged-side-margin-boxes | page-margin-box/left-right-middle | 38 | Missing | @left-middle and @right-middle page-margin boxes must paint in the side margin bands; catc |
| paged-margin-box-styling | page-margin-box/styling | 25 | Missing | A top-center page margin box applies its own color, background, and font size; catches con |
| paged-running-named-page-margin-box | running-element/named-page-margin- | 23 | Missing | position:running() referenced from a named @page chapter margin box appears only on chapte |
| paged-gcpm-string-element-last | running-header/string-last-element | 19 | Missing | string(section,last) and element(head,last) in page margin boxes must use the last heading |
| paged-footnote-styling-pseudos | footnote/styling-and-pseudo-elemen | 7 | Extra | @footnote styling plus ::footnote-call and ::footnote-marker custom content affect footnot |
| paged-named-page-margin-box | page-margin-box/named-page-overrid | 6 | Missing | A block assigned page:chapter must use the chapter @page top-center margin box instead of  |
| paged-running-first-margin-box | running-element/first-page-margin- | 6 | Missing | position:running() referenced from @page :first appears in the first-page header only; cat |
| paged-footnote-policy-block | footnote/policy-block | 6 | ColorValue | footnote-policy:block moves the whole owning paragraph with its footnote when the note can |
| paged-element-first-except | running-element/element-first-exce | 6 | Missing | element(head, first-except) suppresses the running element on the page where it first appe |
| paged-string-set-attr-start | running-header/string-set-attr-sta | 5 | Missing | string-set with attr() feeds string(section,start) in the page header; catches margin head |
| paged-footnote-display-compact | footnote/display-compact | 5 | GeometryShift | footnote-display:compact places fitting short footnotes inline in the footnote area; catch |
| paged-footnote-policy-inline | footnote/policy-line-display-inlin | 4 | ColorValue | footnote-policy:line with footnote-display:inline keeps the owning line with its notes and |
| paged-footnote-counter-reset | footnote/counter-reset | 3 | Extra | counter-reset:footnote changes generated footnote call and marker numbers; catches hard-co |
| paged-top-bottom-margin-boxes | page-margin-box/top-bottom-literal | 2 | Missing | Default @top-left, @top-center, @top-right, and @bottom-center page margin boxes render li |
| paged-first-margin-box-override | page-margin-box/first-page-overrid | 1 | Missing | @page :first replaces the default top-center header on page one only; catches engines that |
| paged-pages-counter-immutable | page-counter/pages-counter-immutab | 1 | Missing | counter-reset:pages in the page context must not alter counter(pages), which remains the t |
| paged-page-counter-reset-seven | page-counter/counter-reset-page-se | 1 | Missing | @page counter-reset:page 7 changes the origin of counter(page) in margin boxes; catches re |
| paged-page-counter-increment-two | page-counter/counter-increment-pag | 1 | Missing | @page counter-increment:page 2 changes counter(page) in margin boxes; catches renderers th |
| paged-corner-margin-box-position | page-margin-box/top-left-corner-po | 1 | Extra | @top-left-corner occupies the page corner while @top-left starts at the content edge; catc |

### filters (32)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| r2-backdrop-filter-blur-stripes | backdrop-filter/backdrop-blur | 100 | Missing | backdrop-filter blur must sample content behind the element: A translucent panel overlays  |
| r2-filter-color-interpolation-filters-srgb | color-interpolation-filters/color- | 83 | ColorValue | color-interpolation-filters must switch SVG filter color space: Two identical muted boxes  |
| r2-filter-blur-text-glyphs | filter: blur() on text/text-glyph- | 73 | Missing | blur() on text glyphs needs raster/filter coverage: Large text with filter: blur(3px) shou |
| r2-filter-before-mask-image-order | filter compositing order with mask | 64 | Extra | filter must be applied before mask-image: A blurred red rectangle is masked by a hard half |
| r2-filter-invalid-function-chain | filter value grammar with an unkno | 63 | Extra | Invalid filter functions must invalidate the whole declaration: A red block has filter: bl |
| r2-filter-missing-url-invalidates-chain | filter: url(#missing) blur()/missi | 59 | Extra | Unresolved filter url() must ignore the entire filter chain: A filter list starts with an  |
| filter-grayscale-group-descendants | filter: grayscale()/group-descenda | 55 | ColorValue | filter: grayscale(1) applies to a parent source graphic including colored text and a child |
| filter-url-svg-gaussian-blur | filter: url()/svg-fegaussianblur | 53 | Missing | filter:url(#id) references an inline SVG feGaussianBlur primitive; catches URL-filter impl |
| r2-filter-drop-shadow-chain-position | filter: drop-shadow() followed by  | 47 | Missing | drop-shadow() must obey authored chain position: A dark drop shadow is followed by brightn |
| r2-filter-drop-shadow-color-first | filter: drop-shadow(<color> <lengt | 45 | Missing | drop-shadow() accepts color before the lengths: The color token appears first in drop-shad |
| r2-filter-drop-shadow-rounded-css-alpha | filter: drop-shadow() on rounded C | 40 | Missing | drop-shadow() on CSS boxes follows the alpha shape, not a rectangular box-shadow: A circul |
| r2-filter-blur-visual-overflow-layout | filter: blur() visual overflow/vis | 36 | Missing | blur() visual overflow must paint outside the border box without moving layout: A blurred  |
| r2-filter-blur-group-descendants | filter: blur() on a container grou | 36 | ColorValue | blur() must apply to the whole element subtree: A container with a blue background, a yell |
| r2-filter-stacking-context-negative-z | filter establishes stacking contex | 36 | ColorValue | filter-created stacking contexts must contain negative z-index descendants: A filtered par |
| r2-filter-url-feblend-multiply | filter: url(#id) with feBlend mult | 34 | ColorValue | SVG feBlend must composite primitive inputs: An inline SVG filter referenced from CSS cont |
| filter-drop-shadow-currentcolor | filter: drop-shadow()/default-curr | 28 | ColorValue | drop-shadow() without a color uses currentColor for the shadow; catches parsers that defau |
| r2-filter-url-feoffset | filter: url(#id) with feOffset/feo | 18 | ColorValue | SVG feOffset must move filtered output: An inline SVG filter referenced from CSS contains  |
| filter-chain-order-contrast-blur | filter: chained/ordered-contrast-b | 18 | ColorValue | contrast(5) blur(8px) and blur(8px) contrast(5) produce different edge softness; catches i |
| r2-filter-url-fecolormatrix-channel-swap | filter: url(#id) with feColorMatri | 17 | ColorValue | SVG feColorMatrix arbitrary matrices must be honored: An inline SVG filter referenced from |
| r2-filter-url-fecomponenttransfer | filter: url(#id) with feComponentT | 17 | ColorValue | SVG feComponentTransfer must apply channel transfer functions: An inline SVG filter refere |
| r2-filter-url-fecomposite-in | filter: url(#id) with feComposite  | 17 | ColorValue | SVG feComposite must support Porter-Duff operators: An inline SVG filter referenced from C |
| r2-filter-url-fecolormatrix-luminance-alpha | filter: url(#id) with feColorMatri | 17 | ColorValue | SVG feColorMatrix luminanceToAlpha must affect alpha: An inline SVG filter referenced from |
| r2-filter-invert-group-descendants | filter: invert() on a container gr | 15 | ColorValue | invert() must filter descendant text and child paint: A container with a solid background, |
| r2-filter-url-femorphology-dilate | filter: url(#id) with feMorphology | 14 | ColorValue | SVG feMorphology must dilate/erode the source alpha: An inline SVG filter referenced from  |
| r2-filter-contrast-group-descendants | filter: contrast() on a container  | 12 | ColorValue | contrast() must filter descendant text and child paint: A container with a solid backgroun |
| r2-filter-brightness-group-descendants | filter: brightness() on a containe | 12 | ColorValue | brightness() must filter descendant text and child paint: A container with a solid backgro |
| r2-filter-hue-rotate-group-descendants | filter: hue-rotate() on a containe | 12 | ColorValue | hue-rotate() must filter descendant text and child paint: A container with a solid backgro |
| r2-filter-url-fedropshadow | filter: url(#id) with feDropShadow | 10 | ColorValue | SVG feDropShadow primitive must render inside filter url(): An inline SVG filter reference |
| r2-filter-url-femerge | filter: url(#id) with feMerge/feme | 9 | ColorValue | SVG feMerge must paint multiple primitive outputs: An inline SVG filter referenced from CS |
| r2-filter-url-feturbulence-displacement | filter: url(#id) with feTurbulence | 6 | ColorValue | Procedural SVG filter primitives must affect static output: An inline SVG filter reference |
| r2-filter-saturate-group-descendants | filter: saturate() on a container  | 1 | Missing | saturate() must filter descendant text and child paint: A container with a solid backgroun |
| r2-filter-before-clip-path-order | filter compositing order with clip | 1 | ColorValue | filter must be applied before clip-path: A blurred red square is clipped to a circle; the  |

### effects (29)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| r2-background-blend-mode-color | background-blend-mode/color | 100 | ColorValue | Non-separable background-blend-mode:color is required: A red/blue gradient background blen |
| r2-background-blend-mode-color-burn | background-blend-mode/color-burn | 100 | ColorValue | background-blend-mode:color-burn needs direct coverage: A two-layer solid linear-gradient  |
| r2-background-blend-mode-color-dodge | background-blend-mode/color-dodge | 100 | ColorValue | background-blend-mode:color-dodge needs direct coverage: A two-layer solid linear-gradient |
| r2-background-blend-mode-darken | background-blend-mode/darken | 100 | ColorValue | background-blend-mode:darken needs direct coverage: A two-layer solid linear-gradient back |
| r2-background-blend-mode-difference | background-blend-mode/difference | 100 | ColorValue | background-blend-mode:difference needs direct coverage: A two-layer solid linear-gradient  |
| r2-background-blend-mode-exclusion | background-blend-mode/exclusion | 100 | ColorValue | background-blend-mode:exclusion needs direct coverage: A two-layer solid linear-gradient b |
| r2-background-blend-mode-hard-light | background-blend-mode/hard-light | 100 | ColorValue | background-blend-mode:hard-light needs direct coverage: A two-layer solid linear-gradient  |
| r2-background-blend-mode-hue | background-blend-mode/hue | 100 | ColorValue | Non-separable background-blend-mode:hue is required: A red/blue gradient background blends |
| r2-background-blend-mode-luminosity | background-blend-mode/luminosity | 100 | ColorValue | Non-separable background-blend-mode:luminosity is required: A red/blue gradient background |
| r2-background-blend-mode-screen | background-blend-mode/screen | 100 | ColorValue | background-blend-mode:screen needs direct coverage: A two-layer solid linear-gradient back |
| r2-background-blend-mode-soft-light | background-blend-mode/soft-light | 100 | ColorValue | background-blend-mode:soft-light needs direct coverage: A two-layer solid linear-gradient  |
| mix-blend-mode-luminosity | mix-blend-mode/luminosity | 100 | ColorValue | mix-blend-mode: luminosity blends a yellow source with a blue backdrop; catches unsupporte |
| background-blend-mode-overlay | background-blend-mode/overlay | 100 | ColorValue | background-blend-mode: overlay blends black/white gradient halves with a purple background |
| r2-background-blend-mode-list-matching | background-blend-mode comma-separa | 75 | ColorValue | background-blend-mode lists must match background layers: Two background image layers use  |
| r2-mix-blend-mode-inline-svg | mix-blend-mode on inline SVG conte | 55 | ColorValue | mix-blend-mode must apply to inline SVG/replaced content: An inline SVG red circle blends  |
| r2-mix-blend-mode-color | mix-blend-mode/color | 53 | Extra | Non-separable mix-blend-mode:color is required: A red square overlaps a blue/gray backdrop |
| r2-mix-blend-mode-saturation | mix-blend-mode/saturation | 53 | Extra | Non-separable mix-blend-mode:saturation is required: A red square overlaps a blue/gray bac |
| r2-background-blend-mode-lighten | background-blend-mode/lighten | 50 | ColorValue | background-blend-mode:lighten needs direct coverage: A two-layer solid linear-gradient bac |
| r2-background-blend-mode-saturation | background-blend-mode/saturation | 50 | ColorValue | Non-separable background-blend-mode:saturation is required: A red/blue gradient background |
| r2-box-shadow-inset-blur-radius | box-shadow inset blur with border- | 48 | ColorValue | inset box-shadow blur must fade inward inside rounded corners: A deterministic solid-color |
| r2-isolation-isolate-descendant-blend | isolation/isolate-descendant-blend | 40 | ColorValue | isolation:isolate must constrain descendant blending: Two identical groups contain a red m |
| r2-text-shadow-blur-rgba | text-shadow blur with rgba color/b | 39 | ColorValue | text-shadow blur with alpha color must preserve soft transparent edges: A large ParitySans |
| r2-mix-blend-mode-hue | mix-blend-mode/hue | 34 | Extra | Non-separable mix-blend-mode:hue is required: A red square overlaps a blue/gray backdrop u |
| r2-opacity-isolates-descendant-blend | opacity group isolation with mix-b | 25 | ColorValue | Opacity-created stacking contexts must isolate inner blending: A parent with opacity:.99 c |
| r2-text-shadow-decoration-line | text-shadow shadows text decoratio | 20 | Missing | text-shadow must shadow text decorations too: A large ParitySans text run isolates text-sh |
| r2-mix-blend-mode-text-difference | mix-blend-mode on text/text-differ | 11 | Missing | mix-blend-mode must apply to text content: Large white text with mix-blend-mode:difference |
| r2-box-shadow-spread-blur-radius | box-shadow positive spread with bl | 5 | ColorValue | box-shadow spread plus blur and radius must grow the rounded shadow shape: A deterministic |
| r2-text-shadow-currentcolor-default | text-shadow currentColor default/c | 3 | GeometrySize | text-shadow omitted color resolves to currentColor: A large ParitySans text run isolates t |
| r2-text-shadow-multiple-order | text-shadow multiple shadow paint  | 3 | GeometryShift | multiple text-shadows must paint first listed shadow on top: A large ParitySans text run i |

### backgrounds-borders (26)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| background-clip-text-gradient | background-clip/text | 98 | Extra | background-clip:text clips a gradient background to glyph shapes; catches engines that map |
| background-position-edge-offsets | background-position/four-value-edg | 93 | ColorValue | background-position: right 20px bottom 15px anchors a tile from named edges; catches one/t |
| background-origin-border-box | background-origin/border-box | 93 | ColorValue | background-origin:border-box positions a tile from the outer border edge; catches implemen |
| background-repeat-space-round | background-repeat/space-round | 89 | ColorValue | background-repeat: space round distributes columns and rounds row tile height; catches par |
| background-blend-mode-list-matching | background-blend-mode/layer-list-m | 88 | ColorValue | Comma-separated background-blend-mode values keep the top red tile normal while multiplyin |
| background-blend-mode-multiply-image | background-blend-mode/multiply-ima | 84 | ColorValue | background-blend-mode:multiply blends a blue image layer with a yellow background color; c |
| background-attachment-local-scroll-canvas | background-attachment/local-scroll | 75 | Extra | background-attachment:local positions a bottom stripe against the scrollable content canva |
| html-background-suppresses-body-canvas | root-body-canvas-background-preced | 64 | ColorValue | An html background paints the canvas and prevents body background propagation; catches eng |
| border-width-four-value-shorthand | border-width/four-value-shorthand | 50 | ColorValue | border-width four-value shorthand assigns visibly different widths to every side; catches  |
| border-color-four-value-shorthand | border-color/four-value-shorthand | 41 | ColorValue | border-color four-value shorthand assigns a distinct color to each side; catches single-co |
| background-size-auto-length | background-size/auto-length-intrin | 38 | ColorValue | background-size: auto 60px preserves the image intrinsic ratio; catches implementations th |
| border-image-gradient-slice | border-image/gradient-slice | 37 | ColorValue | border-image paints a linear-gradient source into a transparent physical border; catches e |
| background-attachment-fixed-viewport | background-attachment/fixed-viewpo | 37 | Missing | background-attachment:fixed anchors a split gradient to the page viewport; catches element |
| border-style-3d-bevels | border-style/groove-ridge-inset-ou | 31 | ColorValue | groove, ridge, inset, and outset render with 3D bevel polarity; catches unsupported keywor |
| box-shadow-inset-elliptical-radius | box-shadow/inset-elliptical-radius | 25 | ColorValue | An inset spread shadow follows an elliptical border-radius inner curve; catches rectangula |
| background-clip-padding-box-elliptical-radius | background-clip/padding-box-ellipt | 24 | Missing | background-clip:padding-box follows elliptical rounded corners; catches scalar radius clip |
| background-clip-list-matching | background-clip/layer-list-matchin | 23 | ColorValue | Comma-separated background-clip values clip only the top layer to content-box over a blue  |
| background-image-image-set | background-image/image-set | 20 | Missing | background-image:image-set() paints its selected candidate; catches parsers that drop imag |
| background-layer-order-raster-gradient | background-layer-order/gradient-ov | 18 | ColorValue | A top red gradient layer covers a bottom blue raster at the same corner; catches fixed slo |
| background-position-xy-longhands | background-position/x-y-longhands | 13 | Missing | background-position-x and background-position-y independently place a tile at fixed offset |
| border-style-four-value-shorthand | border-style/four-value-shorthand | 9 | ColorValue | border-style four-value shorthand assigns solid, dashed, dotted, and double styles to diff |
| background-origin-list-matching | background-origin/layer-list-match | 7 | Missing | Comma-separated background-origin values position two image layers from different boxes; c |
| box-shadow-spread-nonuniform-radius | box-shadow/spread-nonuniform-radiu | 6 | Extra | A spread-only outer box-shadow preserves four unequal corner radii; catches scalar-radius  |
| box-shadow-elliptical-radius | box-shadow/elliptical-percentage-r | 5 | Extra | A pill-shaped border-radius:50% casts a hard oval shadow; catches scalar rounded-rectangle |
| background-shorthand-multiple-layers-color | background-shorthand/multiple-laye | 5 | ColorValue | The background shorthand accepts multiple image layers plus a final background color; catc |
| background-image-nonuniform-radius-clip | background-image/nonuniform-radius | 2 | Extra | A raster background is clipped by four unequal border radii; catches unclipped or uniform- |

### text-advanced (20)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| text-advanced-text-wrap-mode-nowrap | text-wrap-mode:nowrap/text-wrap-mo | 46 | Extra | A long phrase with text-wrap-mode:nowrap overflows a narrow box even though white-space is |
| text-advanced-line-break-strict-cjk-punctuation | line-break:strict/line-break-stric | 38 | Missing | A narrow Japanese phrase uses strict line breaking so closing punctuation stays with the p |
| text-advanced-hyphens-manual-soft-hyphen | hyphens/manual-soft-hyphen | 35 | Missing | hyphens:manual honors a soft hyphen break and paints a hyphen at the line end; catches wra |
| text-advanced-line-break-anywhere | line-break:anywhere/line-break-any | 34 | Missing | A no-space punctuation-heavy run fits a very narrow column only when line-break:anywhere c |
| text-advanced-overflow-wrap-anywhere-min-content | overflow-wrap:anywhere min-content | 25 | Missing | Two inline-blocks with the same long word compare anywhere against break-word; anywhere ca |
| text-advanced-text-overflow-rtl-ellipsis | text-overflow:ellipsis with direct | 21 | GeometryShift | An RTL nowrap overflow box should place the ellipsis at the left inline end when content o |
| text-advanced-word-break-keep-all-cjk | word-break:keep-all with CJK text/ | 18 | GeometrySize | A narrow CJK phrase with keep-all should avoid ordinary CJK line breaks and overflow/clamp |
| text-advanced-text-align-match-parent | text-align:match-parent/text-align | 10 | ColorValue | A child inherits a parent's logical end alignment and resolves it against the parent's RTL |
| text-advanced-dir-auto | dir=auto/dir-auto | 7 | Missing | Two boxes use dir=auto; the Hebrew-first row aligns right while the Latin-first row aligns |
| text-advanced-text-align-end-rtl | text-align/end-rtl | 7 | ColorValue | text-align:end in an RTL block aligns the short run to the physical left edge; catches tre |
| text-advanced-unicode-bidi-plaintext | unicode-bidi:plaintext/unicode-bid | 7 | ColorValue | A pre-line block has one English line and one Hebrew line; plaintext chooses base directio |
| text-advanced-writing-mode-sideways-rl | writing-mode:sideways-rl/writing-m | 6 | ColorValue | A sideways-rl block rotates Latin text as a sideways vertical line instead of horizontal t |
| text-advanced-writing-mode-vertical-lr | writing-mode:vertical-lr/writing-m | 6 | ColorValue | A vertical-lr block should set text vertically with columns progressing left-to-right. Cat |
| text-advanced-mixed-script-bidi | mixed-script Unicode bidi visual o | 6 | GeometryShift | A left-to-right paragraph contains a Hebrew segment between Latin words and should reorder |
| text-advanced-direction-rtl-mixed-bidi | direction:rtl basic inline paragra | 6 | GeometryShift | An RTL block containing Hebrew followed by Latin starts from the right edge and orders the |
| text-advanced-text-combine-upright-digits | text-combine-upright:digits 2/text | 5 | GeometryShift | A vertical date combines two-digit numbers horizontally inside a single vertical character |
| text-advanced-text-align-last-center | text-align-last:center/text-align- | 5 | ColorValue | A justified paragraph has a short final line that should be centered by text-align-last:ce |
| text-advanced-writing-mode-vertical-rl-columns | writing-mode/vertical-rl-columns | 4 | ColorValue | writing-mode:vertical-rl lays Latin inline text sideways from the top of the right column; |
| text-advanced-text-orientation-upright | text-orientation:upright/text-orie | 4 | ColorValue | A vertical-rl box with text-orientation:upright stacks upright Latin letters instead of ro |
| text-advanced-white-space-collapse-preserve | white-space-collapse:preserve/whit | 2 | ColorValue | A normal wrapping line uses the longhand white-space-collapse:preserve so three spaces bet |

### backgrounds-gradients (16)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| linear-gradient-color-hint | linear-gradient/interpolation-hint | 100 | ColorValue | A standalone 20% color interpolation hint moves the red/blue midpoint left; catches gradie |
| repeating-linear-gradient-px-stops | repeating-linear-gradient/length-s | 100 | ColorValue | repeating-linear-gradient with 12px hard color-stop ranges creates fixed-width stripes; ca |
| multiple-gradient-layers | multiple-backgrounds/two-gradient- | 100 | ColorValue | Two gradient background layers use independent size, position, repeat, and paint order; ca |
| linear-gradient-alpha-stops | linear-gradient/alpha-stops | 96 | ColorValue | Transparent-to-opaque red gradient composites over a blue parent; catches gradient painter |
| conic-gradient-color-hint | conic-gradient/color-hint | 87 | Missing | A standalone conic-gradient color hint shifts the angular interpolation midpoint late in t |
| radial-gradient-color-hint | radial-gradient/color-hint | 87 | Missing | A standalone radial-gradient color hint shifts the red/blue interpolation midpoint toward  |
| repeating-radial-gradient-px-ranges | repeating-radial-gradient/length-r | 86 | Missing | repeating-radial-gradient with 10px two-position stop ranges creates crisp rings; catches  |
| linear-gradient-oklab-interpolation | linear-gradient/oklab-interpolatio | 85 | Missing | linear-gradient() with `in oklab` interpolates in Oklab rather than rejecting the gradient |
| linear-gradient-calc-stops | linear-gradient/calc-color-stop-po | 82 | Missing | linear-gradient color stops accept calc() length-percentage positions around the midpoint; |
| radial-gradient-edge-offset-position | radial-gradient/edge-offset-positi | 66 | ColorValue | radial-gradient() accepts a four-value edge-offset at-position from right and bottom; catc |
| background-origin-content-box-gradient | background-origin/content-box-grad | 64 | Extra | background-origin:content-box anchors a fixed-size gradient tile at the content edge; catc |
| conic-gradient-edge-offset-position | conic-gradient/edge-offset-positio | 10 | ColorValue | conic-gradient() accepts a four-value edge-offset at-position from right and bottom; catch |
| radial-gradient-two-position-percent-stops | radial-gradient/two-position-perce | 8 | GeometryShift | radial-gradient two-position percentage stops create crisp yellow, red, and blue rings; ca |
| radial-gradient-closest-corner-offcenter | radial-gradient/closest-corner-off | 4 | ColorValue | An off-center radial-gradient(circle closest-corner) sizes the hard transition by nearest- |
| radial-gradient-farthest-corner-offcenter | radial-gradient/farthest-corner-of | 2 | GeometryShift | An off-center radial-gradient(circle farthest-corner) sizes the hard transition by farthes |
| conic-gradient-turn-rad-stops | conic-gradient/turn-radian-stop-un | 1 | Extra | conic-gradient color-stop positions accept turn and rad angle units in the same list; catc |

### generated-content (14)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| generated-content-target-counter-page | target-counter(attr(href), page)/t | 100 | Missing | A link to a later section appends that target's page number through target-counter(attr(hr |
| generated-content-first-line-line-height | ::first-line line-height/first-lin | 65 | Missing | The first line gets a 54px line-height while later lines keep 24px, creating extra vertica |
| generated-content-first-line-font-size | first-line/font-size-geometry | 58 | Missing | ::first-line font-size and line-height change the first formatted line geometry; catches i |
| generated-content-nested-q-quote-depth | open-quote/close-quote quote depth | 33 | Missing | Nested q elements use generated open-quote/close-quote with two quote pairs; the inner quo |
| generated-content-no-open-quote-depth | no-open-quote/no-close-quote depth | 27 | Extra | A hidden opener increments quote depth before a visible nested quote, so the visible quote |
| generated-content-running-element-margin | position:running() and element()/r | 20 | Missing | A small heading is captured with position:running(head) and reused by @page top-left via e |
| generated-content-first-line-letter-spacing | ::first-line letter-spacing/first- | 16 | GeometryShift | Only the first formatted line gets 5px letter spacing, changing where that line wraps rela |
| generated-content-target-text | target-text(attr(href))/target-tex | 14 | Missing | A cross-reference link appends the heading text of its target using target-text(attr(href) |
| generated-content-string-set-running-header | string-set and string()/string-set | 12 | Extra | A heading stores its text in a named running string and the page top-center margin prints  |
| generated-content-leader-function | content:leader('.')/leader-functio | 12 | Extra | A table-of-contents row inserts a dotted leader between a title and a page number. Catches |
| generated-content-before-list-item-marker | ::before{display:list-item}/before | 8 | GeometryShift | A block's ::before pseudo is display:list-item with list-style-type:square, producing a ma |
| generated-content-first-line-text-transform | ::first-line text-transform/first- | 6 | GeometryShift | A wrapped paragraph uppercases only the first formatted line through ::first-line{text-tra |
| generated-content-first-letter-leading-quote | ::first-letter punctuation unit/fi | 3 | Missing | A paragraph begins with an opening quote; ::first-letter background and color should apply |
| generated-content-first-letter-leading-digit | ::first-letter numeric first-lette | 2 | Missing | A paragraph starts with a digit and ::first-letter should style that digit before the rest |

### lists-counters (13)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| lists-counters-marker-font-size | ::marker font-size/marker-font-siz | 100 | Missing | A list styles only ::marker with a 36px font-size while item text remains 20px. Catches: A |
| lists-counters-counter-style-cyclic | @counter-style cyclic symbols/coun | 82 | Missing | A custom @counter-style named checks cycles through two literal symbols for list markers.  |
| lists-counters-list-style-type-string | list-style-type:<string>/list-styl | 81 | Missing | A ul sets list-style-type:'-- ' so each marker is a literal two-dash string instead of a d |
| lists-counters-counter-list-item-after | counter(list-item)/counter-list-it | 81 | Missing | Each li appends generated text using counter(list-item), matching its ordered-list marker  |
| lists-counters-list-item-increment-step | counter-increment:list-item 2/list | 80 | Missing | An ordered list increments list-item by 2 so markers advance 2, 4, 6. Catches: A list rend |
| lists-counters-ol-reversed | <ol reversed>/ol-reversed | 79 | Missing | A three-item ordered list with the reversed attribute should render 3, 2, 1 markers. Catch |
| lists-counters-li-value | <li value>/li-value | 79 | Missing | The middle li value='7' changes that marker and subsequent items continue from 8. Catches: |
| lists-counters-marker-content-counter | ::marker{content:...}/marker-conte | 78 | Missing | An ordered list replaces default decimal markers with a generated counter wrapped in squar |
| lists-counters-counter-style-pad-prefix-suffix | @counter-style extends decimal pad | 76 | Missing | A custom counter style extends decimal with pad:2 '0', prefix '[' and suffix '] '. Catches |
| lists-counters-list-style-type-cjk-decimal | list-style-type:cjk-decimal/list-s | 74 | Missing | An ordered list uses cjk-decimal markers, which render CJK decimal digits rather than west |
| lists-counters-marker-side-match-parent | marker-side:match-parent/marker-si | 34 | Extra | A vertical list sets marker-side:match-parent so markers use the parent's side rather than |
| lists-counters-counter-style-negative | @counter-style negative descriptor | 8 | ColorValue | A generated counter starts at -2 and uses a custom negative descriptor to wrap negative va |
| lists-counters-counter-set | counter-set/assign-existing-counte | 3 | GeometryShift | counter-set:item 7 assigns the existing counter so generated labels read 1, 7, 8; catches  |

### fonts-advanced (12)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| fonts-advanced-font-shorthand | font shorthand/font-shorthand | 33 | Missing | A single font shorthand declaration sets italic bold 28px/36px ParitySerif in one declarat |
| fonts-advanced-font-synthesis-none | font-synthesis:none/font-synthesis | 16 | Extra | A local @font-face exposes only a regular face; font-weight:bold with font-synthesis:none  |
| fonts-advanced-font-face-size-adjust | @font-face size-adjust descriptor/ | 15 | Missing | An @font-face alias applies size-adjust:150% to ParitySans, making it visibly larger than  |
| fonts-advanced-font-size-adjust | font-size-adjust:0.8/font-size-adj | 14 | Missing | Two same-size lines compare default font sizing with font-size-adjust:0.8, which changes u |
| fonts-advanced-font-face-unicode-range | @font-face unicode-range/font-face | 13 | Missing | Two @font-face rules share a family; uppercase A comes from ParitySerif and uppercase B fr |
| fonts-advanced-font-face-style-descriptor | @font-face font-style descriptor/f | 13 | Missing | A shared @font-face family maps normal to ParitySans and italic to ParitySerif; font-style |
| fonts-advanced-font-face-weight-descriptor | @font-face font-weight descriptor/ | 13 | Extra | Two @font-face rules share a family name but map normal to ParitySerif and bold to ParityS |
| fonts-advanced-font-kerning-none | font-kerning:none/font-kerning-non | 12 | Missing | The AVAV pair is rendered with and without kerning so the no-kerning row is visibly wider/ |
| fonts-advanced-font-face-local-src | @font-face src:local()/font-face-l | 12 | Missing | An @font-face rule aliases local ParitySerif and then uses that alias for a line of text.  |
| fonts-advanced-font-variant-ligatures-none | font-variant-ligatures:no-common-l | 7 | Extra | A serif line disables common ligatures through the high-level font-variant-ligatures prope |
| fonts-advanced-font-weight-relative | font-weight:bolder/lighter/font-we | 3 | Missing | Nested spans use bolder and lighter relative to a 600-weight parent, producing visibly dif |
| fonts-advanced-small-caps-synthesis | font-variant-caps/small-caps-synth | 2 | GeometryShift | font-variant-caps:small-caps synthesizes smaller uppercase forms for lowercase letters; ca |

### multicol (12)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| multicol-writing-mode-vertical-rl | column-progression/writing-mode-ve | 100 | Extra | writing-mode:vertical-rl changes multicol line boxes and column progression instead of ren |
| multicol-break-before-column | break-before/column | 64 | Missing | break-before:column inside a column-fill:auto multicol container must move the block to th |
| multicol-break-after-column | break-after/column | 64 | Missing | break-after:column inside a fixed-height multicol container starts the following block at  |
| multicol-direction-rtl-columns | column-progression/direction-rtl | 58 | Extra | direction:rtl places the first multicol column on the right side of the container; catches |
| multicol-break-before-avoid-column | break-before/avoid-column | 49 | Missing | break-before:avoid-column keeps a heading with the following paragraph in the same column  |
| multicol-break-inside-avoid-column | break-inside/avoid-column | 48 | Missing | break-inside:avoid-column pushes a straddling card whole to the next column; catches colum |
| multicol-column-fill-balance-all | column-fill/balance-all | 45 | Extra | column-fill:balance-all balances every page fragment of a multicol flow, not only the fina |
| multicol-forced-overflow-columns | column-fragmentation/forced-overfl | 44 | Missing | Forced break-after:column values can create an actual third overflow column beyond the two |
| multicol-overflow-columns-auto | column-fragmentation/overflow-colu | 33 | Extra | A fixed-height two-column container with column-fill:auto creates additional overflow colu |
| multicol-column-rule-empty-suppressed | column-rule/empty-adjacent-columns | 23 | Extra | Column rules must not paint beside empty columns when only the first column has content; c |
| multicol-column-gap-percentage | column-gap/percentage | 22 | Extra | column-gap:10% must resolve against the multicol container and narrow the three columns ac |
| multicol-column-rule-wider-than-gap | column-rule/wider-than-gap | 19 | Extra | A column rule wider than the gap is centered in the gap and overlaps columns without widen |

### color-opacity (11)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| r2-background-color-currentcolor | background-color/background-curren | 100 | ColorValue | currentColor must resolve in background-color: A fixed-size color swatch isolates parsing/ |
| r2-color-function-display-p3 | color(display-p3 ...)/color-displa | 100 | ColorValue | display-p3 colors must gamut-map to output sRGB: A fixed-size color swatch isolates parsin |
| r2-color-function-srgb | color(srgb ...)/color-srgb | 100 | ColorValue | color(srgb ...) must parse as a predefined RGB color: A fixed-size color swatch isolates p |
| r2-color-function-srgb-linear | color(srgb-linear ...)/color-srgb- | 100 | ColorValue | color(srgb-linear ...) must convert linear-light components: A fixed-size color swatch iso |
| r2-color-function-xyz-d65 | color(xyz-d65 ...)/color-xyz-d65 | 100 | ColorValue | XYZ predefined colors must convert to output sRGB: A fixed-size color swatch isolates pars |
| color-oklch | color-format/oklch | 100 | ColorValue | CSS Color 4 oklch() background color converts to output sRGB; catches legacy-only parsers  |
| r2-color-lab | lab()/lab | 100 | ColorValue | lab() colors must convert to sRGB: A fixed-size color swatch isolates parsing/conversion/c |
| r2-color-lch | lch()/lch | 100 | ColorValue | lch() colors must convert to sRGB: A fixed-size color swatch isolates parsing/conversion/c |
| r2-color-oklab | oklab()/oklab | 100 | ColorValue | oklab() colors must convert to sRGB: A fixed-size color swatch isolates parsing/conversion |
| r2-opacity-group-flatten-alpha-overlap | opacity group flattening with alph | 45 | ColorValue | opacity on a group with alpha children must flatten before compositing: A parent opacity:. |
| opacity-text-glyph-group | opacity/text-group | 24 | Missing | Overlapping glyphs in one opacity:.5 element flatten before compositing; catches per-glyph |

### inline-text (10)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| inline-text-text-indent-percent | text-indent/percent-containing-blo | 34 | Extra | text-indent:50% indents only the first formatted line by half the containing block width;  |
| inline-text-text-decoration-wavy | text-decoration-style:wavy/text-de | 20 | Missing | A large underlined word requests a red wavy underline instead of the default solid rule. C |
| inline-text-text-decoration-thickness-length | text-decoration-thickness:6px/text | 19 | Missing | A large underlined word sets a 6px decoration thickness, visibly thicker than the default  |
| inline-text-text-decoration-multiple-lines | text-decoration-line:underline ove | 17 | Missing | A span requests underline and overline at the same time. Catches: A single-keyword text-de |
| inline-text-text-underline-offset-length | text-underline-offset:10px/text-un | 10 | Missing | An underlined word requests a 10px underline offset, leaving a large gap below the baselin |
| inline-text-vertical-align-percent | vertical-align:<percentage>/vertic | 9 | ColorValue | An inline-block square uses vertical-align:50% and shifts upward by half its own line-heig |
| inline-text-vertical-align-text-bottom | vertical-align/text-bottom | 8 | Extra | vertical-align:text-bottom aligns to the parent text bottom inside a tall line box; catche |
| inline-text-vertical-align-length | vertical-align/length | 7 | ColorValue | vertical-align:18px shifts an inline-block above the surrounding baseline; catches keyword |
| inline-text-break-spaces-trailing-wrap | white-space:break-spaces trailing  | 6 | ColorValue | A narrow preserved-space line wraps after visible spaces before the next word instead of h |
| inline-text-word-spacing-percent | word-spacing:<percentage>/word-spa | 3 | Missing | Two identical text rows compare normal spacing with word-spacing:200%, making every word g |

### transforms (10)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| transforms-origin-z-offset | transform-3d/transform-origin-z-of | 84 | Missing | A three-value transform-origin with a Z offset changes the rotateY hinge; catches ignoring |
| transforms-rotate3d-axis | transform-3d/rotate3d-axis | 49 | ColorValue | rotate3d(1,1,0,62deg) rotates around an arbitrary 3D axis under perspective; catches dropp |
| transforms-perspective-rotate-x | transform-3d/perspective-rotate-x | 42 | ColorValue | perspective() rotateX() projects the box as a 3D quadrilateral; catches 2D flattening, per |
| transforms-scale3d-rotate-y | transform-3d/scale3d-rotate-y | 41 | ColorValue | scale3d's Z component changes the projected shape of a rotateY plane; catches dropping sca |
| transforms-matrix3d-translate | transform-3d/matrix3d-translate | 37 | ColorValue | matrix3d() translation components move the green box down and right from the red reference |
| transforms-preserve-3d-depth-order | transform-3d/preserve-3d-depth-ord | 33 | Missing | transform-style:preserve-3d lets an earlier translateZ child paint above a later flat sibl |
| transforms-translate3d-perspective | transform-3d/translate3d-perspecti | 33 | ColorValue | translate3d(x,y,z) combines planar movement with perspective enlargement; catches dropping |
| transforms-individual-transform-box-content | individual-transform/translate-rot | 27 | ColorValue | Individual translate and rotate properties compose around transform-box:content-box with t |
| transforms-perspective-translate-z | transform-3d/perspective-translate | 27 | ColorValue | A parent perspective makes a translateZ child render larger than the red reference; catche |
| transforms-perspective-origin | transform-3d/perspective-origin | 7 | ColorValue | perspective-origin changes the projection center for otherwise identical rotateY boxes; ca |

### overflow-clipping (8)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| overflow-clip-float-no-bfc | overflow/clip-no-float-containment | 74 | Extra | overflow:clip clips overflow but does not establish a new formatting context, so a followi |
| overflow-line-clamp-two-lines | line-clamp/two-line-webkit-clamp | 64 | ColorValue | -webkit-line-clamp:2 with vertical box layout clips static text to two lines before the re |
| overflow-logical-inline-clip | overflow/logical-inline-block-axes | 40 | Extra | overflow-inline:clip and overflow-block:visible clip only inline-axis overflow in horizont |
| overflow-visible-clip-axis | overflow/visible-x-clip-y | 38 | Missing | overflow-x:visible with overflow-y:clip preserves horizontal overflow while clipping verti |
| overflow-scrollbar-gutter-both-edges | scrollbar-gutter/stable-both-edges | 19 | ColorValue | scrollbar-gutter:stable both-edges reserves symmetric inline gutters around the child; cat |
| overflow-clip-margin | overflow-clip-margin/clip-edge-ext | 11 | Missing | overflow:clip with overflow-clip-margin:18px lets the child bleed past the box edge; catch |
| overflow-scrollbar-gutter-stable | scrollbar-gutter/stable | 11 | ColorValue | scrollbar-gutter:stable reserves inline gutter space in a non-overflowing auto scroll cont |
| overflow-axis-visible-hidden-coercion | overflow/visible-hidden-axis-coerc | 1 | Extra | overflow-x:visible plus overflow-y:hidden coerces the x axis to non-visible clipping; catc |

### selectors-cascade (8)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| selectors-cascade-layers-order | cascade-layers/layer-order-over-so | 100 | Missing | @layer reset, base, theme gives the later-declared theme layer priority over a later sourc |
| selectors-cascade-defined | pseudo-class/defined | 100 | ColorValue | :defined matches built-in HTML elements, turning the div green; catches missing Selectors  |
| selectors-cascade-dir-rtl | pseudo-class/dir-rtl | 100 | ColorValue | :dir(rtl) matches an element with static dir=rtl; catches missing direction pseudo-class s |
| selectors-cascade-lang-inherited | pseudo-class/lang-inherited | 100 | ColorValue | :lang(en) matches an element inheriting the document language from html lang=en; catches c |
| selectors-cascade-scope-root | pseudo-class/scope-root | 100 | ColorValue | :scope in a stylesheet matches the document scope root so :scope body .box turns the box g |
| selectors-cascade-has-descendant | selectors-4/has-descendant | 100 | ColorValue | :has(.flag) matches a descendant witness nested below the card; catches :has implementatio |
| selectors-has-nth-of-supports | selectors-4/supports-has-nth-child | 29 | ColorValue | @supports selector(), child :has(> .flag), and :nth-child(2 of .flagged) make the intended |
| selectors-cascade-all-initial-reset | cascade/all-initial-reset | 22 | ColorValue | all:initial resets earlier padding before later declarations rebuild the box; catches trea |

### block-box-model (6)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| block-display-contents | display/contents-box-suppression | 100 | Missing | display:contents suppresses the wrapper box, paint, and padding while keeping children in  |
| block-display-inline-grid | display/inline-grid-multi-keyword | 100 | Extra | display:inline grid creates inline-level grid containers whose children lay out in columns |
| block-max-width-min-content | max-width/min-content | 61 | Missing | max-width:min-content constrains a 200px text box to the longest word and wraps the phrase |
| block-min-width-max-content | min-width/max-content | 43 | ColorValue | min-width:max-content expands a narrow box to its unbreakable text width; catches ignoring |
| block-flow-root-float-containment | display/flow-root-float-containmen | 36 | Missing | display:flow-root establishes a BFC whose height contains a floated child before the follo |
| block-display-list-item | display/list-item-marker | 12 | ColorValue | display:list-item on a div generates an inside marker box; catches treating list-item as a |

### positioning (5)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| positioning-inset-shorthand-stretch | inset/shorthand-stretch | 38 | ColorValue | The inset shorthand stretches an absolutely positioned box with inset:20px 30px 40px 50px; |
| positioning-logical-insets-horizontal | inset/logical-insets-horizontal | 33 | ColorValue | Logical inset longhands map to physical top/right/bottom/left in horizontal-tb writing mod |
| positioning-relative-percent-offset | position/relative-percent-offset | 18 | ColorValue | position:relative percentage top/left offsets resolve against the containing block and cov |
| positioning-z-index-negative-behind-flow | z-index/negative-behind-flow | 17 | ColorValue | A positioned z-index:-1 child in a local stacking context stays behind an overlapping in-f |
| positioning-fixed-repeats-pages | position/fixed-repeats-paged-media | 15 | ColorValue | position:fixed header repeats at the top of each printed page; catches treating fixed as a |

### typography (5)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| typography-line-height-em-inheritance | line-height:<length em> inheritanc | 51 | Missing | A parent sets font-size:20px and line-height:2em; a smaller child inherits a 40px computed |
| typography-mixed-inline-font-size-linebox | mixed inline font-size line box me | 41 | Extra | A paragraph with a 42px span in the middle must produce a taller first line than the follo |
| typography-text-emphasis-filled-dot | text-emphasis filled dot/text-emph | 39 | Extra | Japanese text uses filled red emphasis dots above the characters. Catches: An implementati |
| typography-line-height-percent | line-height/percentage-font-size | 30 | Missing | line-height:200% resolves from the element font-size to 40px line boxes; catches percentag |
| typography-initial-letter-two-line | initial-letter:2/initial-letter-tw | 16 | GeometryShift | A paragraph uses p::first-letter{initial-letter:2} to create a dropped initial spanning tw |

### units-values (5)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| units-viewport-svw-svh | viewport-units/svw-svh | 36 | Missing | Small viewport units resolve in static print layout, making a 50svw by 25svh box on a fixe |
| units-lh-line-height | font-relative-length/lh | 25 | ColorValue | The lh unit resolves against the element's computed line-height, so width:4lh with line-he |
| units-cap-height | font-relative-length/cap | 21 | Extra | The cap unit resolves from font cap height and is visibly shorter than a 10em reference; c |
| units-length-ex-x-height | font-relative-length/ex-x-height | 15 | Extra | The ex unit uses the font x-height in layout lengths; catches treating ex as em, unsupport |
| units-ch-layout-width | font-relative-length/ch-width | 7 | Extra | width:20ch in ParityMono uses the zero glyph advance and is wider than a 10em reference; c |

### images-replaced (2)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| svg-inline-gradient-clip-path | inline-svg/gradient-clip-path | 90 | ColorValue | An inline SVG gradient-filled rect clipped by a clipPath paints only inside the circle; ca |
| img-object-position-far-edge-length | object-position/far-edge-length-co | 48 | ColorValue | object-position:right 20px bottom 10px offsets a cover-fitted image from far edges; catche |

### interactions (1)

| id | feature/sub | diff% | class | defect |
|---|---|--:|---|---|
| interactions-supports-flow-root-containment | supports/flow-root-query-and-layou | 26 | Missing | Interaction: @supports(display:flow-root) must both query true and lay out flow-root float |
