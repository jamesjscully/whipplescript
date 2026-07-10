# Experimentation / improve subsystem — tracker

Status: active (registered 2026-07-10 at the v0.4 close-out — the release
banner's "improve/evals" half was previously pre-ADR research notes with no
tracker; this makes it OPEN INTENT under tracker discipline). Design SSOTs:
`experimentation-subsystem-research-note.md` (surface + identification +
evidence-plane IFC + consent/canary postures) and `improve-design-note.md`
(the improve loop). This tracker holds the build intent and the release-scope
decision; the notes hold the design.

## Settled ground (design passes with Jack, 2026-07-01 → 2026-07-04)

Not re-opened here: the `gauge` + `mark` surface (pin/suppose/settle/
evidence/why; ambient); identification/quasi-experimental posture;
evidence-plane IFC (§18.2 — scope ⊥ clearance; no-new-readers); GEPA
rejected; HOLDOUT = 20% / floor-2 / k=3; CAMPAIGN surface =
naming-is-partition + a `campaign` declaration; consent/canary = shadow
posture, canary = RCT.

## ⚑ Release-scope decision (Jack — blocks the v0.4 cut definition)

- [ ] ⚑ **Does improve/evals build into 0.4, or does 0.4 re-cut as the
      version-control release with improve/evals as 0.5's banner?**

      *Analysis (2026-07-10; decision is Jack's):*

      **For building into 0.4.** (a) The release plan (settled 2026-07-05)
      names both halves, and the versioned workspace is the substrate the
      evidence plane and campaign partitions were designed to ride — landing
      them together exercises the new store immediately. (b) The settled
      ground above is substantial; the surface design is not starting cold.

      **Against (for re-cutting).** (a) Four design questions are still
      OPEN and Jack-held (campaign scope, proposer leakage, the utility
      model, the cross-workspace evidence door) — under the
      discussion-before-design-choices house rule these need real design
      passes, not autonomous resolution, so the build cannot start
      immediately regardless. (b) The version-control half is complete,
      gated, and independently valuable today; holding it hostage to a
      pre-ADR subsystem inverts the ship-what's-done discipline the 0.2/0.3
      cuts followed. (c) 0.3 set the precedent: scope decisions at the cut
      (Jack, 2026-07-09) trimmed to what was verified.

      **Lean:** re-cut 0.4 as the version-control release; improve/evals
      becomes 0.5's banner with the four open design questions as the first
      agenda. The runbook in `release-checklist.md` stages the mechanical
      steps either way.

## Build items (gated on the scope decision + the four open design passes)

- [ ] The four open design passes with Jack: campaign scope; proposer
      leakage; the utility model; the cross-workspace evidence door.
      (Discussion-first; each closes into the research note.)
- [ ] ADR for the `gauge`/`mark`/`campaign` surface (formalizes the settled
      ground; the registry note "pre-ADR" clears here).
- [ ] Evidence-plane build: `gauge` + `mark` declarations, ambient evidence
      rows, §18.2 IFC refinement (scope ⊥ clearance, no-new-readers).
- [ ] Improve loop v1: HOLDOUT (20% / floor-2 / k=3) + campaign
      partitions over the versioned workspace's branch/workstream tiers.
- [ ] Consent/canary posture: shadow default; canary = RCT.
