import { build, context } from "esbuild";
import type { BuildOptions } from "esbuild";
import { copyFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

/** Build static assets or serve a local preview with watched JS and CSS. */
async function buildWebsite(): Promise<void> {
  const directory = fileURLToPath(new URL(".", import.meta.url));
  const preview = process.argv.includes("--serve");
  const options: BuildOptions = {
    absWorkingDir: directory,
    entryPoints: ["card-animation.ts", "style.css"],
    bundle: true,
    minify: !preview,
    format: "iife",
    target: ["es2020"],
    outdir: "dist",
    logLevel: "info",
  };

  await mkdir(new URL("dist/", import.meta.url), { recursive: true });
  for (const file of ["index.html", "favicon.svg", "CNAME"]) {
    await copyFile(
      new URL(file, import.meta.url),
      new URL(`dist/${file}`, import.meta.url),
    );
  }

  if (preview) {
    const builder = await context(options);
    await builder.watch();
    await builder.serve({ servedir: "dist", host: "127.0.0.1", port: 8765 });
  } else {
    await build(options);
  }
}

await buildWebsite();
