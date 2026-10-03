import { frameAt } from "./card-frames.ts";

/** Start the decorative card loop and preserve its position while hidden. */
function startCardAnimation(art: HTMLPreElement): void {
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let elapsed = 0;
  let startedAt = performance.now();
  let timer: number | undefined;
  let running = false;

  /** Draw only changed frames and sleep while a face is held still. */
  function draw(): void {
    const frame = frameAt(
      elapsed + (running ? performance.now() - startedAt : 0),
    );
    if (art.textContent !== frame.text) art.textContent = frame.text;
    if (running) timer = window.setTimeout(draw, frame.delay);
  }

  /** Suspend background updates and honor changes to the motion preference. */
  function updatePlayback(): void {
    window.clearTimeout(timer);
    if (running) elapsed += performance.now() - startedAt;
    if (reducedMotion.matches) elapsed = 0;
    running = !document.hidden && !reducedMotion.matches;
    startedAt = performance.now();
    draw();
  }

  document.addEventListener("visibilitychange", updatePlayback);
  reducedMotion.addEventListener("change", updatePlayback);
  updatePlayback();
}

/** Required display for the card animation. */
const art = document.querySelector<HTMLPreElement>("#card-art");
if (!art) throw new Error("The page is missing its card illustration.");
startCardAnimation(art);
