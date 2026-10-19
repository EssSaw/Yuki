# Visual direction: Watercolor Sketchbook

## Confirmed direction

The user rejected the Dream Arcade prototype as too neon/cyberpunk and sharp.
Current direction supersedes that treatment: warm paper, pale watercolor pigment
washes, hand-painted accents and soft organic panels with occasional brushed edges.
Use muted powder blue and dusty rose; no jagged cutouts, thick black outlines,
near-black/neon technical palette or condensed all-caps display typography.

Retain original Persona-inspired button responsiveness, not its geometry/assets:
snappy soft brushstroke hover reveals, subtle lift and gentle slide/fade transitions.
Keep elegant readable typography and calm text surfaces; texture stays out of dense
code/document text. Mature and dreamy, not babyish or a generic pastel dashboard.

Themes respond to workspace intent: technical uses muted slate-blue structure on
light paper; creative uses lighter rose washes. Conversation-aware AI suggestions
are future scope and must not unexpectedly change the interface.

Two complementary layouts are accepted: an expressive chat home and a denser project
workspace; avatar small/collapsible during focused work. Short directional transitions,
no perpetual animation behind reading/code, reduced-motion support and no hidden-window
animation. Sound is optional/on by default when implemented, as specified below.

## Skill selection and application

For UI design work, discover and inspect compatible maintained skills rather than
choosing by popularity alone. Use a visual-direction skill for composition and a
separate accessibility/usability checklist for validation. Record selected sources
and inspect license/scripts before installation; do not add runtime UI dependencies
or transmit private code just to use a skill.

Selected reviewed guidance: [Anthropic frontend-design](https://github.com/anthropics/skills/blob/main/skills/frontend-design/SKILL.md)
for original brief-led composition and restrained motion, complemented by
[Vercel web-design-guidelines](https://github.com/vercel-labs/agent-skills/blob/main/skills/web-design-guidelines/SKILL.md)
for semantic controls, keyboard focus, contrast and reduced-motion checks. Apply
these constraints during UI work; no downloaded scripts or runtime dependencies are
needed. Browser-only SSR/URL guidance is not automatically applicable to this desktop shell.

## Approved prototype

Implement one chat-home/navigation prototype for review before extending the redesign
to full notes/canvas/code layouts. Soft organic navigation and painted hover feedback
replace angled panels/overlapping labels; all text stays level and unobscured.
Use a large abstract portrait placeholder on chat home and compact/collapsible character
framing during active work, until an original/licensed portrait is supplied.

User delegated palette mapping: Projects default to Technical (muted slate blue), Notes &
canvas to Creative (light pink), Schedule/Settings to balanced neutral, and Chat to
balanced blue/pink. Auto follows navigation; Technical/Creative manual overrides stay
locked until changed, with gentle transitions and no content-driven changes while typing.
Persist the choice when local settings storage is available; do not claim persistence
until implemented. Future AI suggestions must respect manual overrides.

UI sound is optional and on by default when implemented, with a visible mute control.
Sound assets/implementation are not required for the first visual prototype; show no
misleading active sound control. Motion must respect reduced-motion preferences.

Implementation refinement: use open paper-like surfaces instead of enclosing every
message/control in cards. Maintain one shared token/reset stylesheet and cohesive
screen styles; replace obsolete rules rather than piling overrides onto them. Broad
pigment washes and subtle static paper grain live behind content, with level typography,
compact toolbars and quiet boundaries across chat, settings and project browsing.

## Approved Visual Checkpoint (Superseding Revisions)

The approved visual checkpoint refines the watercolor sketchbook into a quiet, coherent desktop slice:
- **Scenery & Observatory**: Sage scenery with muted blue and dusty pink sky (`/landscape.png`)
  remains visible across the app, anchored to `right center` (`background-size: cover`) so the
  observatory on the right remains preserved during responsive cropping.
- **Continuous Reading Surface**: Reading surfaces use a continuous, mostly opaque cool ivory
  soft-edge surface (cool-ivory gradient and diffuse edges, without a panel border)
  to ensure quiet, high-legibility reading while letting the scenery frame the space.
- **Open Assistant & Subtle Blue User Messages**: Assistant messages sit open directly on the
  ivory surface without cards, borders, or glass. User messages use a subtle muted blue wash
  (`var(--muted-blue-bg)`) with soft rounded corners. Glassmorphism and card stacks are eliminated.
- **Compact Humanist Typography & Wordmark**: Standardized on compact humanist UI typography
  (`Candara`, `Calibri`, `Trebuchet MS` fallback), discarding oversized serif headings. Brand uses a small,
  expressive Yuki wordmark (`y.` mark with refined type).
- **Clean Controls & Illustrated Highlights**: Icons and controls are crisp geometric SVGs,
  not hand-drawn sketches. Only the landscape art and stepped highlight silhouettes are illustrated.
- **Stepped Silhouette & Static Targets**: Hover and focus feedback uses a stepped few-silhouette
  continuous three-silhouette redraw (`720ms steps(1, end) infinite`), while button bounds, text, and click targets stay completely static.
  Active nav items have a static backing wash. Reduced-motion preferences force immediate static display.
- **Collapsed Icon Rail Default**: The sidebar defaults to a compact 64px icon rail, persisting the
  user's expand/collapse choice in `localStorage` (`yuki.sidebar_collapsed`). All navigation and
  sections remain accessible in both states. Appearance controls live in Settings, not navigation.
- **Retained Functional Slice**: All model selection, 9router connection settings, streaming chat,
  cancellation, retry, and history deletion behaviors are preserved intact.

UI design and implementation use a Hard UI subagent, with primary-assistant coordination,
integration and validation; see the model-selection rule in [AGENTS.md](../../AGENTS.md).
This supersedes the earlier primary-assistant-only UI preference.
The first checkpoint requires desktop and narrow-width visual
review before propagation, commits or pushes. The integrated title bar provides native drag, minimize, maximize/restore and close
controls, with visible action failures. Native chrome is disabled. Subtle pointer-driven
landscape parallax and short workspace entrance transitions respect reduced motion.
User accepted the overall look, readability pass and expanded-sidebar fit on 2026-10-19.
Full native-control and narrow-width interaction acceptance remain to be documented.

Apply reviewed skill guidance and verify the working prototype before committing.
The user will review this slice before the design is propagated to other work surfaces.
