# DR-0031 — The identity boundary

Status: accepted (2026-06-28). Resolves the open question behind E7 (whip-agent
acts-for-user, DR-0028 D3): how does WhippleScript know *which user* an agent serves,
so the agent's authority can be scoped to that user's clearances — without
WhippleScript becoming an identity or data-governance product.

## The two goals, and why they are not in tension

1. WhippleScript is primarily a **programming language**, not an identity/governance
   solution.
2. It must be **usable inside enterprises**, which means working with their existing
   identity and permission systems.

These conflict only if WhippleScript tries to *be* an identity system. They are
orthogonal once WhippleScript **consumes** an identity assertion and never
**authenticates** one. The language's job is flow control; identity is delegated.

## Decision: three layers, WhippleScript owns one

| Layer | Question | Owner |
| --- | --- | --- |
| **Identity** | Who is this principal, and prove it | The enterprise — OS login, SSO/IdP, directory |
| **Access control** | May this principal *open* this resource | The OS / the resource's own ACLs |
| **Flow control** | May this *data* go *there* | **WhippleScript** |

Flow control is the new capability (neither an IdP nor file permissions provide it).
The other two layers are solved problems WhippleScript rides on, and they **compose**:
an agent runs as an OS principal, so the OS already gates which files it may open
(access control, free); the `kind:address` a grant binds (DR-0027 E5, e.g.
`file:/srv/crm.db`) *is* that OS/enterprise resource; WhippleScript adds the flow
dimension on top.

## The identity seam

There is exactly one narrow interface — **`current_principal()` → an identity
assertion** — with pluggable backends, and one mapping from the principal to an
acts-for role. The mapping already exists in the governance grammar: a
`party <identity> : <Role>` line (parsed today, currently ignored). IT writes the
party lines; the backend supplies the identity; the resolved role becomes the agent's
authority **ceiling** for the IFC check (acts-for that role, never beyond — D3).

Backends, by trust boundary:

- **`os` (default).** The process's effective user (uid/username on Unix, SID on
  Windows). On a managed/AD-joined host this *is* the enterprise identity — the OS
  authenticated it at login, and the same principal gives file access control for
  free. Zero new infrastructure. The production form of the `WHIPPLESCRIPT_GOV_ADMIN`
  = sudo pattern.
- **`env` / launcher-passed.** A trusted parent (web gateway, scheduler, K8s) that
  has already authenticated the end-user passes the identity in (env var / header).
  WhippleScript trusts its launcher, exactly as enterprise apps trust the reverse
  proxy that did the SSO. This is also the dev/v0 backend.
- **`token` (future).** Validate an OIDC/JWT against the IdP's JWKS and read a
  `groups`/`roles` claim — direct IdP integration without a fronting gateway.
  Designed-for, not built now.

`os` and `env` are the same seam with different backends; `env` is the cheap v0, not
a throwaway. The trust of each backend is explicit and matches an existing enterprise
boundary (OS login / trusted gateway / IdP signature) — WhippleScript adds no new
trust root of its own.

## What WhippleScript will NOT build

No identity provider, session store, credential vault, auth protocol, or user
database. The moment it does, it is a worse identity product than the dedicated tools
*and* a worse language. The line: it accepts an identity **assertion** and maps it to
a role; everything upstream of that assertion belongs to the enterprise.

## Consequences for E7

E7 splits along the seam:

- **Enforcement (acts-for-user ceiling) — built for real.** The resolved role is the
  agent's authority ceiling; the IFC check already has acts-for and the `party` map.
  This is the language's job and is sound regardless of backend.
- **Identity source — pluggable.** Ship the **`os`** and **`env`** backends now;
  document **`token`** as the future backend. E7 is then DONE for the controlled /
  managed-host deployment (the common case) and cleanly extensible to direct-IdP.

The unauthenticated-`env` caveat is recorded, not hidden: `env` is a *scoping*
mechanism for a controlled deployment, not a security boundary against a malicious
local actor — identical to the `gov-admin` token. `os` and `token` are the boundaries
that hold against that adversary.

## Implementation sketch (the follow-on slice)

1. A `principal` module: `current_principal(backend) -> Principal`, backends `os`
   (read effective user) and `env` (`WHIPPLESCRIPT_PRINCIPAL`), selected by config
   (default `os`); `token` left as a documented extension point.
2. Envelope: use the already-parsed `party <identity> : <Role>` lines as an
   identity→role map; resolve the current principal to a role (unmapped principal =
   the public bottom, fail-closed).
3. Check/runtime: thread the principal's role as the **authority ceiling** — a turn
   may read only what the role's clearance permits, and a flow to a sink the role
   cannot reach is a violation (the same acts-for machinery, with the principal as an
   additional reader-authority constraint).
4. Tests: party→role resolution; an agent scoped to a low-clearance principal is
   refused a high-clearance read/flow; `os` and `env` backends.
