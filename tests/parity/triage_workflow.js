export const meta = {
  name: 'parity-nonpass-triage',
  description: 'Visually triage every non-PASS parity fixture (fresh render vs Chrome ref): real-bug vs cross-engine floor',
  phases: [
    { title: 'Triage', detail: 'one agent per fixture: Read out+ref PNG, classify' },
    { title: 'Confirm', detail: 'adversarially re-check each real-bug verdict' },
  ],
}

// args: array of { id, cat }  (category folder + fixture id)
const FX = typeof args === 'string' ? JSON.parse(args) : args
const ROOT = '/home/frederic/IdeaProjects/ironpress/tests/parity'

const TRIAGE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'verdict', 'confidence', 'what_differs', 'likely_cause'],
  properties: {
    id: { type: 'string' },
    verdict: { type: 'string', enum: ['real-bug', 'floor'] },
    confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
    what_differs: { type: 'string', description: 'Concrete visible difference, or "none visible" for floor' },
    likely_cause: { type: 'string', description: 'For real-bug: the CSS feature + suspected render/layout cause. Empty for floor.' },
  },
}

const CONFIRM_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['id', 'still_real', 'reason'],
  properties: {
    id: { type: 'string' },
    still_real: { type: 'boolean' },
    reason: { type: 'string' },
  },
}

function triagePrompt(fx) {
  const out = `${ROOT}/out/${fx.cat}/${fx.id}.png`
  const ref = `${ROOT}/refs/${fx.cat}/${fx.id}.png`
  return `You are triaging a single HTML→PDF parity fixture. Two PNGs render the SAME page at the SAME size:
- CANDIDATE (ironpress, the engine under test): ${out}
- REFERENCE (Chrome, ground truth): ${ref}

Read BOTH images and compare them carefully. Decide whether the candidate has a REAL rendering bug or is only separated from the reference by irreducible cross-engine floor.

REAL-BUG = a difference a human would notice and call wrong: missing/extra element, wrong color or fill, wrong position/size/alignment, wrong text wrapping or line count, clipped/overflowing content, a band/edge/corner painted differently, gradient/shadow/border rendered wrong, etc.

FLOOR = the two images look the SAME to a human; differences are only sub-pixel anti-aliasing on glyph/shape edges, 1px rounding, or faint AA fringes. Text that wraps to the same lines with the same glyphs at the same places is FLOOR even if edges shimmer.

Be decisive and specific. If real, name the concrete difference and the CSS feature involved (the fixture id "${fx.id}" hints at it) plus your best guess at the cause. Do not invent differences that aren't visible.

Return ONLY the structured verdict. id MUST be exactly "${fx.id}".`
}

function confirmPrompt(t, fx) {
  const out = `${ROOT}/out/${fx.cat}/${fx.id}.png`
  const ref = `${ROOT}/refs/${fx.cat}/${fx.id}.png`
  return `A first reviewer flagged this fixture as a REAL rendering bug. Your job is to REFUTE that — be skeptical. Default to still_real=false unless the difference is unmistakable.

Fixture: ${fx.id}
First reviewer said differs: ${t.what_differs}
First reviewer's suspected cause: ${t.likely_cause}

- CANDIDATE (ironpress): ${out}
- REFERENCE (Chrome): ${ref}

Read BOTH images. Is the claimed difference actually visible and actually wrong (not just AA/sub-pixel/rounding)? If the images look the same to a human, set still_real=false. Only confirm still_real=true if a human would clearly call the candidate wrong. id MUST be exactly "${fx.id}".`
}

phase('Triage')
const results = await pipeline(
  FX,
  (fx) => agent(triagePrompt(fx), { label: `triage:${fx.id}`, phase: 'Triage', schema: TRIAGE_SCHEMA }).then((t) => ({ t, fx })),
  ({ t, fx }) => {
    if (!t || t.verdict !== 'real-bug') return { t, fx, confirm: null }
    return agent(confirmPrompt(t, fx), { label: `confirm:${fx.id}`, phase: 'Confirm', schema: CONFIRM_SCHEMA })
      .then((c) => ({ t, fx, confirm: c }))
  },
)

const clean = results.filter(Boolean)
const confirmedReal = clean.filter((r) => r.t && r.t.verdict === 'real-bug' && r.confirm && r.confirm.still_real)
const disputedReal = clean.filter((r) => r.t && r.t.verdict === 'real-bug' && (!r.confirm || !r.confirm.still_real))
const floor = clean.filter((r) => r.t && r.t.verdict === 'floor')

log(`Triage done: ${confirmedReal.length} confirmed real-bug, ${disputedReal.length} disputed, ${floor.length} floor`)

return {
  confirmed_real: confirmedReal.map((r) => ({ id: r.t.id, cat: r.fx.cat, what: r.t.what_differs, cause: r.t.likely_cause, confidence: r.t.confidence })),
  disputed: disputedReal.map((r) => ({ id: r.t.id, what: r.t.what_differs, reason: r.confirm ? r.confirm.reason : 'no confirm' })),
  floor_ids: floor.map((r) => r.t.id),
}
