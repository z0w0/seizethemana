import assert from "node:assert/strict";
import { build } from "esbuild";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import { frameAt } from "./card-frames.ts";

/** Exercise the shipped browser entry point without adding a DOM library. */
const bundle = await build({
  entryPoints: [fileURLToPath(new URL("card-animation.ts", import.meta.url))],
  bundle: true,
  write: false,
  format: "iife",
});

/** Browser state needed to verify timers, user controls, and motion settings. */
function animationBrowser(reduce = false) {
  const art = { textContent: frameAt(0).text };
  const toggle = Object.assign(new EventTarget(), {
    hidden: true,
    textContent: "Pause animation",
  });
  const motion = Object.assign(new EventTarget(), { matches: reduce });
  const document = Object.assign(new EventTarget(), {
    hidden: false,
    querySelector: (selector: string) =>
      selector === "#card-art" ? art : toggle,
  });
  let time = 0;
  let timer: (() => void) | undefined;
  runInNewContext(bundle.outputFiles[0].text, {
    document,
    performance: { now: () => time },
    window: {
      matchMedia: () => motion,
      clearTimeout: () => {
        timer = undefined;
      },
      setTimeout: (callback: () => void) => {
        timer = callback;
        return 1;
      },
    },
  });
  return {
    art,
    toggle,
    motion,
    document,
    get scheduled() {
      return timer !== undefined;
    },
    advance(milliseconds: number) {
      time += milliseconds;
      const callback = timer;
      timer = undefined;
      callback?.();
    },
  };
}

test("the initial card stays stable and the user can pause and resume", () => {
  const browser = animationBrowser();
  assert.equal(browser.art.textContent, frameAt(0).text);
  assert.equal(browser.toggle.hidden, false);
  browser.advance(4300);
  const paused = browser.art.textContent;
  browser.toggle.dispatchEvent(new Event("click"));
  assert.equal(browser.toggle.textContent, "Play animation");
  assert.equal(browser.scheduled, false);
  browser.advance(10000);
  assert.equal(browser.art.textContent, paused);
  browser.toggle.dispatchEvent(new Event("click"));
  assert.equal(browser.toggle.textContent, "Pause animation");
  browser.advance(1280);
  assert.match(browser.art.textContent, /Lightning Bolt/);
});

test("reduced motion stays static and responds to preference changes", () => {
  const browser = animationBrowser(true);
  assert.equal(browser.toggle.hidden, true);
  assert.equal(browser.scheduled, false);
  browser.advance(10000);
  assert.equal(browser.art.textContent, frameAt(0).text);
  browser.motion.matches = false;
  browser.motion.dispatchEvent(new Event("change"));
  assert.equal(browser.toggle.hidden, false);
  browser.advance(5580);
  assert.match(browser.art.textContent, /Lightning Bolt/);
  browser.motion.matches = true;
  browser.motion.dispatchEvent(new Event("change"));
  assert.equal(browser.art.textContent, frameAt(0).text);
  assert.equal(browser.scheduled, false);
});

test("hidden pages stop timers without losing playback or user pause", () => {
  const browser = animationBrowser();
  browser.advance(2000);
  browser.document.hidden = true;
  browser.document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(browser.scheduled, false);
  browser.advance(20000);
  browser.document.hidden = false;
  browser.document.dispatchEvent(new Event("visibilitychange"));
  browser.advance(3580);
  assert.match(browser.art.textContent, /Lightning Bolt/);
  browser.toggle.dispatchEvent(new Event("click"));
  browser.document.hidden = true;
  browser.document.dispatchEvent(new Event("visibilitychange"));
  browser.document.hidden = false;
  browser.document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(browser.scheduled, false);
  assert.equal(browser.toggle.textContent, "Play animation");
});
