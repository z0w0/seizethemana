import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { frameAt } from "./card-frames.ts";

/** Authored HTML also serves users who cannot load the animation script. */
const html = await readFile(new URL("index.html", import.meta.url), "utf8");

test("the no-script card is the exact first 28-by-40 animation frame", () => {
  const card = html.match(
    /<pre id="card-art" aria-hidden="true">([\s\S]*?)<\/pre>/,
  );
  assert.ok(card);
  // HTML drops a line break immediately after the opening pre tag.
  assert.equal(
    card[1].replace(/^\n/, "").replace(/&#32;/g, " "),
    frameAt(0).text,
  );
});

test("page and social metadata use the same title and product description", () => {
  const title = html.match(/<title>(.*?)<\/title>/)?.[1];
  const description = html.match(/name="description"\s+content="([^"]+)"/)?.[1];
  assert.ok(title);
  assert.ok(description);
  for (const attribute of ["og:title", "twitter:title"]) {
    assert.ok(html.includes(`${attribute}" content="${title}"`));
  }
  for (const attribute of ["og:description", "twitter:description"]) {
    assert.ok(html.includes(`${attribute}"\n      content="${description}"`));
  }
  const structured = html.match(
    /<script type="application\/ld\+json">([\s\S]*?)<\/script>/,
  );
  assert.ok(structured);
  const data = JSON.parse(structured[1]);
  for (const item of data["@graph"]) {
    assert.equal(item.description, description);
    assert.equal(item.url, "https://seizethemana.com/");
  }
});
