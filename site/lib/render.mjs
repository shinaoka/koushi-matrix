// Renders one locale's page. Every link is relative so the site works under the
// /koushi-matrix/ project path; external links are absolute https URLs.
import { LOCALES } from "./content.mjs";

export const SITE_URL = "https://shinaoka.github.io/koushi-matrix/";
export const REPO_URL = "https://github.com/shinaoka/koushi-matrix";
export const SCENES = ["main", "notifications", "search-mixed", "search-width", "settings", "languages", "rich"];

const LANGUAGE_NAMES = {
  en: "English",
  ja: "日本語",
  "zh-Hans": "简体中文",
  "zh-Hant": "繁體中文",
  ko: "한국어"
};

const BLOG = {
  why: { ja: "https://shinaoka.github.io/blog/naze-matrix-ja/", other: "https://shinaoka.github.io/blog/why-matrix/" },
  setup: {
    ja: "https://shinaoka.github.io/blog/matrix-encryption-setup-checklist-ja/",
    other: "https://shinaoka.github.io/blog/matrix-encryption-setup-checklist/"
  }
};

const DOWNLOAD = `${REPO_URL}/releases/latest/download`;

const GITHUB_ICON = '<svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>';

const PAIN_POINTS = [
  { id: "accounts" },
  { id: "email", shot: "notifications" },
  { id: "utd" },
  { id: "search", shot: "search-mixed" },
  { id: "multilingual", shot: "search-width" },
  { id: "ime" },
  { id: "photos" },
  { id: "help" }
];

const AI_CAN = ["search", "unread", "reply", "attachments", "wait"];
const AI_CONTROL = ["grant", "scope", "flags", "plain"];
const AI_TRY = ["unread", "search", "reply"];
// Shown verbatim in every locale; it mirrors the Claude Code command in docs/help/ai-assistants.md.
const AI_COMMAND = "claude mcp add --scope user koushi -- /path/to/koushi-mcp --client-name claude-code";

const FEATURES = ["e2ee", "layout", "timeline", "rich", "files", "desktop", "keywords", "spaces", "updates", "diagnostics"];

const SETTINGS_ROWS = [
  "app.appearance",
  "app.preferences",
  "app.keyboard",
  "app.search",
  "account.account",
  "account.sessions",
  "account.notifications",
  "account.privacy",
  "account.encryption",
  "account.search"
];

export function escapeHtml(text) {
  return String(text)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function localePath(locale) {
  return locale === "en" ? "" : `${locale}/`;
}

export function renderPage({ locale, strings, staleKeys = [], englishStrings = strings, release }) {
  const stale = new Set(staleKeys);
  const t = (key, vars = {}) => {
    const source = stale.has(key) ? englishStrings : strings;
    if (typeof source[key] !== "string") {
      throw new Error(`${locale}: missing string ${key}`);
    }
    let text = source[key];
    for (const [name, value] of Object.entries(vars)) {
      text = text.replaceAll(`{${name}}`, value);
    }
    return escapeHtml(text);
  };
  const up = locale === "en" ? "" : "../";
  const shot = (scene, altKey, cls = "") => {
    const src = `${up}screenshots/${locale}/${scene}.png`;
    // The hero image is above the fold, so it loads eagerly; the rest wait for scroll.
    const loading = cls === "hero-shot" ? "eager" : "lazy";
    return `<figure class="shot ${cls}"><a href="${src}"><img src="${src}" alt="${t(altKey)}" loading="${loading}" decoding="async"></a></figure>`;
  };
  const blog = (kind) => (locale === "ja" ? BLOG[kind].ja : BLOG[kind].other);
  const help = (page) => `${REPO_URL}/blob/${escapeHtml(release.tag)}/docs/help/${page}`;

  const alternates = LOCALES.map(
    (other) => `<link rel="alternate" hreflang="${other}" href="${SITE_URL}${localePath(other)}">`
  ).join("\n    ");
  const switcher = LOCALES.map((other) =>
    other === locale
      ? `<li><span aria-current="page">${LANGUAGE_NAMES[other]}</span></li>`
      : `<li><a href="${up}${localePath(other) || "./"}" hreflang="${other}" lang="${other}">${LANGUAGE_NAMES[other]}</a></li>`
  ).join("");

  let shotIndex = 0;
  const painItems = PAIN_POINTS.map(
    ({ id, shot: scene }) => `
        <article class="pain${scene ? ` has-shot${shotIndex++ % 2 === 1 ? " flip" : ""}` : ""}">
          <div class="pain-text">
            <p class="pain-problem">${t(`pain.${id}.problem`)}</p>
            <p class="pain-answer">${t(`pain.${id}.answer`)}</p>
          </div>
          ${scene ? shot(scene, `pain.${id}.shot`) : ""}
        </article>`
  ).join("");

  return `<!doctype html>
<html lang="${locale}">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>${t("meta.title")}</title>
    <meta name="description" content="${t("meta.description")}">
    <link rel="canonical" href="${SITE_URL}${localePath(locale)}">
    ${alternates}
    <link rel="alternate" hreflang="x-default" href="${SITE_URL}">
    <meta property="og:title" content="${t("meta.title")}">
    <meta property="og:description" content="${t("meta.description")}">
    <meta property="og:image" content="${SITE_URL}screenshots/${locale}/main.png">
    <link rel="icon" href="${up}favicon-32.png" sizes="32x32">
    <link rel="stylesheet" href="${up}style.css">
  </head>
  <body>
    <header class="site-header">
      <a class="brand" href="${up}${localePath(locale) || "./"}"><img src="${up}koushi-photon.svg" alt="" width="32" height="32"><span>Koushi</span></a>
      <div class="header-end">
        <nav aria-label="${t("nav.language")}"><ul class="languages">${switcher}</ul></nav>
        <a class="github-link" href="${REPO_URL}">${GITHUB_ICON}<span>GitHub</span></a>
      </div>
    </header>
    ${stale.size > 0 ? `<p class="translating-note" role="note">${t("translating.note")}</p>` : ""}
    <main>
      <section class="hero">
        <div class="hero-text">
          <h1>${t("hero.tagline")}</h1>
          <p class="lede">${t("hero.lede")}</p>
          <p class="cta">
            <a class="button" href="${DOWNLOAD}/Koushi-macos-arm64.dmg">${t("hero.download")}</a>
            <a class="button secondary" href="${REPO_URL}">${GITHUB_ICON}${t("hero.github")}</a>
            <span class="version">${t("hero.version", { version: release.version })}</span>
          </p>
          <p class="other-platforms">${t("hero.other")}
            <a href="${DOWNLOAD}/Koushi-windows-x64-unsigned.exe">${t("hero.windows")}</a> ·
            <a href="${DOWNLOAD}/Koushi-linux-x64.AppImage">${t("hero.appimage")}</a> ·
            <a href="${DOWNLOAD}/Koushi-linux-x64.deb">${t("hero.deb")}</a> ·
            <a href="${DOWNLOAD}/Koushi-linux-x64.rpm">${t("hero.rpm")}</a> ·
            <a href="${REPO_URL}/releases/latest">${t("hero.allReleases")}</a>
          </p>
        </div>
        ${shot("main", "shot.main", "hero-shot")}
      </section>

      <section class="matrix card" id="matrix">
        <h2>${t("matrix.heading")}</h2>
        <p>${t("matrix.body")}</p>
        <p>${t("matrix.account")}</p>
        <ul class="links">
          <li><a href="${blog("why")}">${t("matrix.blog")}</a></li>
          <li><a href="${blog("setup")}">${t("matrix.setup")}</a></li>
        </ul>
      </section>

      <section class="pains" id="why">
        <h2>${t("pain.heading")}</h2>
        <p class="section-lede">${t("pain.lede")}</p>
        ${painItems}
      </section>

      <section class="ai" id="ai">
        <h2>${t("ai.heading")}</h2>
        <p class="section-lede">${t("ai.lede")}</p>
        <div class="ai-columns">
          <div class="card">
            <h3>${t("ai.can.heading")}</h3>
            <ul>
              ${AI_CAN.map((id) => `<li>${t(`ai.can.${id}`)}</li>`).join("\n              ")}
            </ul>
          </div>
          <div class="card">
            <h3>${t("ai.control.heading")}</h3>
            <ul>
              ${AI_CONTROL.map((id) => `<li>${t(`ai.control.${id}`)}</li>`).join("\n              ")}
            </ul>
          </div>
        </div>
        <div class="ai-try">
          <pre class="ai-command"><code>${escapeHtml(AI_COMMAND)}</code></pre>
          <p class="ai-try-heading">${t("ai.try.heading")}</p>
          <ul class="ai-prompts">
            ${AI_TRY.map((id) => `<li>${t(`ai.try.${id}`)}</li>`).join("\n            ")}
          </ul>
        </div>
        <p class="ai-setup">${t("ai.setup")} <a href="${help("ai-assistants.md")}">${t("ai.guide")}</a></p>
      </section>

      <section class="features" id="features">
        <h2>${t("features.heading")}</h2>
        <ul class="feature-grid">
          ${FEATURES.map((id) => `<li>${t(`features.${id}`)}</li>`).join("\n          ")}
        </ul>
        ${shot("rich", "shot.rich", "narrow")}
      </section>

      <section class="languages-section" id="languages">
        <div>
          <h2>${t("languages.heading")}</h2>
          <p>${t("languages.body")}</p>
        </div>
        ${shot("languages", "shot.languages")}
      </section>

      <section class="settings" id="settings">
        <h2>${t("settings.heading")}</h2>
        <p class="section-lede">${t("settings.body")}</p>
        <div class="settings-body">
          <ul class="settings-list">
            ${SETTINGS_ROWS.map((id) => `<li>${t(`settings.${id}`)}</li>`).join("\n            ")}
          </ul>
          ${shot("settings", "shot.settings")}
        </div>
        <p class="ask">${t("settings.ask")} <a href="${help("README.md")}">${t("settings.guide")}</a></p>
      </section>

      <section class="fineprint">
        <div class="card">
          <h2>${t("platforms.heading")}</h2>
          <p>${t("platforms.body")}</p>
        </div>
        <div class="card">
          <h2>${t("missing.heading")}</h2>
          <p>${t("missing.body")}</p>
        </div>
      </section>
    </main>
    <footer class="site-footer">
      <ul>
        <li><a href="https://matrix.to/#/#koushi-matrix:matrix.org">${t("footer.community")}</a></li>
        <li><a href="${REPO_URL}">${t("footer.source")}</a></li>
        <li>${t("footer.license")}</li>
      </ul>
      <p>${t("footer.translation")}</p>
    </footer>
  </body>
</html>
`;
}
