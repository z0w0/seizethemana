import assert from "node:assert/strict";
import test from "node:test";
import { frameAt } from "./card-frames.ts";

test("card faces change through the back and the sequence wraps", () => {
  assert.match(frameAt(0).text, /Sol Ring/);
  assert.match(frameAt(4800).text, /SEIZE THE MANA/);
  assert.match(frameAt(5580).text, /Lightning Bolt/);
  assert.match(frameAt(11160).text, /Birds of Paradise/);
  assert.match(frameAt(16740).text, /Counterspell/);
  assert.match(frameAt(22320).text, /Swords to Plowshares/);
  assert.equal(frameAt(27900).text, frameAt(0).text);
});

test("every face and flip keeps the same stage dimensions", () => {
  for (let elapsed = 0; elapsed < 27900; elapsed += 25) {
    const rows = frameAt(elapsed).text.split("\n");
    assert.equal(rows.length, 28);
    assert.ok(rows.every((row) => row.length === 40));
    assert.ok(frameAt(elapsed).delay > 0);
  }
});

test("the card narrows to an edge before revealing the next face", () => {
  let previousWidth = 34;
  for (let elapsed = 4200; elapsed < 4440; elapsed += 10) {
    const width = frameAt(elapsed).text.split("\n")[1].trim().length;
    assert.ok(width <= previousWidth);
    previousWidth = width;
  }
  const edge = frameAt(4470).text.split("\n").slice(1, -1);
  assert.ok(edge.every((row) => row.trim() === "|"));
});
