# Website

## Run

Run these commands from `site/`. Use Node.js 24 or later:

```sh
npm ci
npm start
```

Open `http://localhost:8765`. TypeScript and CSS changes rebuild automatically.
Restart `npm start` after editing HTML or the favicon.

## Build

```sh
npm run build
```

Open `dist/index.html` to view the production build.

## Deploy

1. In the repository's **Settings → Pages**, select **GitHub Actions** as the source.
2. Set the custom domain to `seizethemana.com` and save it.
3. At your DNS provider, add these records:

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
