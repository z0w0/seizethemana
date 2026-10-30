# Website

The static site uses monospace text, ASCII art, flamingo pink `#ff5c9a`, and
dark ink `#211820`. See [CLI design](../docs/design.md) for shared writing,
color, and accessibility rules.

## Run

Use Node.js 24 or later and npm. CI uses Node.js 24. Run these commands from
`site/`:

```sh
npm ci
npm start
```

Open `http://127.0.0.1:8765`. The preview binds to loopback, not the local
network. TypeScript and CSS changes rebuild automatically. Reload the browser
to see them. Restart `npm start` after editing HTML, the favicon, `CNAME`,
`robots.txt`, or `sitemap.xml`; the script copies these files only at startup.

## Check

```sh
npm run check
npm test
```

The first command checks TypeScript. The second runs the Node test suite.
For copy or layout changes, also check a narrow viewport, keyboard focus,
reduced motion, and the page with scripts disabled. Decorative art must not
hide the install and documentation links.

## Build

```sh
npm run build
```

The build type-checks the source, bundles and minifies TypeScript and CSS,
and copies static files into `dist/`. It does not run tests. Open
`dist/index.html` to inspect the result. Use the local server for browser checks
that need HTTP behavior.

## Deploy

1. In the repository's **Settings → Pages**, select **GitHub Actions** as the source.
2. Set the custom domain to `seizethemana.com` and save it.
3. At your DNS provider, add the GitHub Pages records below. Recheck the
   [GitHub Pages DNS guide](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site)
   before changing a live domain:

Verify domain ownership in GitHub before changing DNS. Do not add wildcard
records. For this Actions deployment, the Pages custom-domain setting controls
the domain; copying `CNAME` into `dist/` does not configure it by itself.

| Type  | Name  | Value             |
| ----- | ----- | ----------------- |
| A     | `@`   | `185.199.108.153` |
| A     | `@`   | `185.199.109.153` |
| A     | `@`   | `185.199.110.153` |
| A     | `@`   | `185.199.111.153` |
| CNAME | `www` | `z0w0.github.io`  |

4. Push the website changes to `main`. The **Deploy website** workflow publishes
   `site/dist/`. You can also run it from the **Actions** tab.
5. Enable **Enforce HTTPS** in Pages settings when GitHub makes it available.

`.github/workflows/pages.yml` runs `npm ci`, `npm test`, and `npm run build`
before publishing. A local preview does not verify DNS, the GitHub Pages domain
setting, or certificate status.
