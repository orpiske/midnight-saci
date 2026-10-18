# Midnight Saci: Visual Identity

Brand guide for building Midnight Saci interfaces (web admin UI first).

## 1. The idea

Midnight Saci is an AI test engineer. The name comes from the **Saci-Pererê**, a trickster from Brazilian folklore who hides things, ties knots and causes small chaos for fun. Our Saci uses that mischief for good: he goes looking for bugs at night so you don't have to.

The brand should feel like **a calm night shift with a mischievous edge**:

- **Nocturnal and calm.** The base is deep midnight blue. The UI is quiet, focused and easy on the eyes during long sessions.
- **Mischievous, never silly.** Personality shows up in small moments: the red cap, glowing eyes, a witty empty state. It never shows up in the core workflow.
- **Precise.** This is an engineering tool. Data, logs and results are clear, monospaced where it helps, and never decorated.

Use the folklore lightly. Don't add cartoon characters, jungle imagery or stereotypical "Brazil" visuals (flags, parrots, carnival).

## 2. Logo

The mark is the Saci reduced to symbols: a **red pointed cap** with its tip curling toward a **crescent moon**, **two glowing yellow eyes** under the brim, and **four stacked arcs below** that suggest his whirlwind (the *redemoinho*). In the full mark it all sits inside a navy circle with a few faint stars.

| Asset | Use |
|---|---|
| `midnight-saci-logo.png` | Full lockup (mark + wordmark + "AI TEST ENGINEER"): login screen, about page |
| `midnight-saci-mark.svg` / `.png` | Round mark: sidebar header, avatar, loading states |
| `midnight-saci-icon.svg` / `.png` | Rounded-square app icon: favicon, PWA icon |

**Wordmark:** "Midnight Saci" set in Bricolage Grotesque ExtraBold (800) with tight tracking (about −0.03em). "Midnight" is in Moonlight and "Saci" is in Cap Red.

**Do**
- Place the logo on Midnight or Surface backgrounds.
- Leave clear space around it equal to at least the height of the cap.
- In a collapsed sidebar, use the mark or icon alone (minimum 24px).

**Don't**
- Recolor the cap to anything other than Cap Red.
- Put the logo on light or busy backgrounds without the navy circle.
- Stretch, rotate, add shadows or add gradients.

## 3. Color

### Brand palette

| Token | Hex | Role |
|---|---|---|
| Midnight | `#0B1026` | App background |
| Night Surface | `#141B3A` | Cards, sidebar, panels (also the mark's circle) |
| Night Raised | `#1C2550` | Hover rows, dropdowns, modals |
| Night Border | `#2A3366` | Borders, dividers |
| Moonlight | `#F2E9D8` | Primary text, warm off-white |
| Dusk | `#B9B3C9` | Secondary text, labels |
| Fog | `#7F7A99` | Placeholder, disabled text (use only at 14px+) |
| **Cap Red** | `#E5402F` | Brand accent: primary buttons, active nav, focus moments |
| Cap Red Hover | `#F0503F` | Primary button hover |
| **Eye Glow** | `#FFD166` | Secondary accent: highlights, "running" state, badges |

**Rules**
- **Cap Red is rare.** Give a screen one primary action and the active navigation state, not a sea of red. If everything is red, nothing stands out.
- **Text on Cap Red must be pure white (`#FFFFFF`).** Moonlight on red doesn't have enough contrast.
- **Don't use Cap Red for small body text** on Midnight (contrast is about 3.8:1). It's fine for text 18px or larger, and for icons and fills.
- **Eye Glow on Midnight has strong contrast** and works for small text, but keep it for highlights.

### Test status colors

Test results are the core of the product, so their colors must be clearly distinct from each other and from the brand red.

| Status | Hex | Notes |
|---|---|---|
| Passed | `#4FD1A5` | Mint-teal |
| Failed | `#FF6B8B` | Rose. **Deliberately not Cap Red**, so a failure never looks like a button and the brand red never feels like an alarm |
| Flaky / Warning | `#FFD166` | Eye Glow |
| Running | `#7AA2FF` | Moonlit blue, with a subtle pulse |
| Skipped / Queued | `#7F7A99` | Fog |

Always pair a status color with an icon and a text label. Never rely on color alone.

### Light theme (optional, secondary)

Dark is the default and primary theme. If you add a light theme: background `#F7F2E8`, surface `#FFFFFF`, text `#0B1026`, secondary text `#4A4560`, border `#E2DACB`. Keep Cap Red as is, and darken the status colors so they reach 4.5:1 on white.

## 4. Typography

| Role | Font | Weights |
|---|---|---|
| Display and headings | **Bricolage Grotesque** (Google Fonts) | 600, 800 |
| UI and body text | **Bricolage Grotesque** | 400, 500 |
| Code, logs, IDs, durations, CLI | **JetBrains Mono** (Google Fonts) | 400, 600 |

Scale for a dense admin UI (px):

| Token | Size / line height | Use |
|---|---|---|
| display | 40 / 44, weight 800, tracking −0.02em | Login, empty-state headlines |
| h1 | 28 / 34, 700 | Page titles |
| h2 | 20 / 28, 600 | Section titles |
| h3 | 16 / 24, 600 | Card titles |
| body | 14 / 22, 400 | Default text |
| small | 12 / 18, 500 | Meta, table headers |
| overline | 11 / 16, JetBrains Mono 600, uppercase, tracking 0.12em | Section labels, like the "AI TEST ENGINEER" tagline |

Use tabular numbers (`font-variant-numeric: tabular-nums`) for counts, durations and percentages.

## 5. Shape, space and depth

- **Spacing:** 4px base. Use the scale 4, 8, 12, 16, 24, 32, 48, 64.
- **Radius:** 6px for inputs and buttons, 10px for cards and panels, 16px for modals, full radius for pills and badges.
- **Borders over shadows.** Separate layers with a 1px Night Border and a step up in surface color, not with drop shadows. Modals may use a soft shadow: `0 16px 48px rgba(0,0,0,0.45)`.
- **Focus ring:** 2px Eye Glow outline with a 2px offset. It's highly visible on every surface.

## 6. Components (direction)

- **Primary button:** Cap Red fill, white text, 6px radius, 36px tall (40px for touch).
- **Secondary button:** transparent background, 1px Night Border, Moonlight text, Night Raised fill on hover.
- **Sidebar:** Night Surface background. The active item gets a Cap Red indicator and Moonlight text. Inactive items use Dusk text.
- **Tables:** dense, 40px rows, hairline row dividers, and Night Raised on row hover. Monospace for IDs, commits and durations.
- **Status badges:** pill shape with a tinted background (status color at about 15% opacity), the status color for text and icon, and an icon plus label.
- **Logs and code blocks:** background `#070A1A` (darker than Midnight), JetBrains Mono 13px, with line numbers in Fog.
- **Charts:** Passed, Failed and Flaky use the status colors. Other series use Moonlit blue `#7AA2FF`, Dusk and Eye Glow. Avoid Cap Red in charts.

## 7. Iconography and imagery

- Use line icons with a 1.5px stroke and rounded caps and joins (for example Lucide or Phosphor "regular").
- Small brand motifs are allowed: a tiny crescent moon for "nightly run" or "scheduled", and whirlwind arcs for "running" or "exploring". Keep them occasional.
- No stock photos, no 3D blobs, no purple-to-pink gradient washes.

## 8. Voice in the UI

Clear first, playful second. Use personality only in empty states, onboarding and success moments, never in errors that block someone's work.

- Empty state: "Nothing broken yet. The Saci is still looking."
- Nightly run finished: "Night shift done. 3 new bugs found before your coffee."
- Error: plain and actionable, with no jokes. For example: "Couldn't reach the test runner. Check that the agent is online."

## 9. Tokens (CSS)

```css
:root {
  /* Surfaces */
  --ms-bg: #0B1026;
  --ms-surface: #141B3A;
  --ms-raised: #1C2550;
  --ms-border: #2A3366;
  --ms-code-bg: #070A1A;

  /* Text */
  --ms-text: #F2E9D8;
  --ms-text-muted: #B9B3C9;
  --ms-text-faint: #7F7A99;
  --ms-on-accent: #FFFFFF;

  /* Brand */
  --ms-accent: #E5402F;        /* Cap Red */
  --ms-accent-hover: #F0503F;
  --ms-glow: #FFD166;          /* Eye Glow */

  /* Status */
  --ms-pass: #4FD1A5;
  --ms-fail: #FF6B8B;
  --ms-flaky: #FFD166;
  --ms-running: #7AA2FF;
  --ms-skipped: #7F7A99;

  /* Type */
  --ms-font-sans: "Bricolage Grotesque", ui-sans-serif, system-ui, sans-serif;
  --ms-font-mono: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, monospace;

  /* Shape */
  --ms-radius-sm: 6px;
  --ms-radius-md: 10px;
  --ms-radius-lg: 16px;
  --ms-focus: 0 0 0 2px var(--ms-bg), 0 0 0 4px var(--ms-glow);
}
```

Google Fonts import:

```html
<link href="https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:opsz,wght@12..96,400;12..96,500;12..96,600;12..96,700;12..96,800&family=JetBrains+Mono:wght@400;600&display=swap" rel="stylesheet">
```
