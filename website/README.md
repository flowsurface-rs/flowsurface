# Flowsurface Website

Marketing / landing site for the Flowsurface open-source orderflow & market structure charting platform.

## 🚀 Project Structure

Key directories:

```text
/
├── public/                # Static assets (favicons, future OG image)
├── src/
│   ├── components/        # Modular sections & layout (Hero, Features, etc.)
│   ├── assets/            # Imported images processed by Astro
│   └── pages/
│       └── index.astro    # Composes page from components
├── styles/                # Global stylesheet(s)
├── TODO.md                # Enhancement task list
└── package.json
```

Primary layout wrapper: `src/components/BaseLayout.astro` centralizes `<html>`, meta, theme initialization, Open Graph & Twitter tags, and a skip link for accessibility.

Component sections live in `src/components/`:
- `Header.astro` navigation + theme toggle
- `Hero.astro` headline & primary CTA
- `Features.astro` interactive feature tabs
- `HowItWorks.astro` 3-step onboarding summary
- `DownloadSection.astro` platform download CTA
- `FinalCTA.astro` closing call to action
- `Footer.astro` site footer

## 🧞 Commands

All commands are run from the project root:

| Command | Action |
| :------ | :----- |
| `npm install` | Install dependencies |
| `npm run dev` | Start dev server (default: `http://localhost:4321`) |
| `npm run build` | Build production site to `./dist/` |
| `npm run preview` | Preview the built site locally |

## 🛠 Development Notes
- Theme preference persisted in `localStorage` under `fs-theme` and applied early to avoid FOUC.
- Open Graph/Twitter meta tags are declared in `BaseLayout.astro`; update image URL once real preview available (`hero-screenshot` task).
- Accessibility: includes skip link + semantic `main`; further contrast & nav improvements tracked in `TODO.md` (`a11y`, `mobile-nav`).

## 🧩 Next Enhancements
See `TODO.md` for prioritized tasks (OG image asset, mobile nav, FAQ, etc.).

## 📄 License
Website content will follow the main project license (add reference/link when repository root license is finalized).


