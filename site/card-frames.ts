/** Content for one illustrated Magic card. */
interface Card {
  name: string;
  cost: string;
  type: string;
  caption: string;
  art: string;
}

/** A complete character-grid frame and its time until the next update. */
interface CardFrame {
  text: string;
  delay: number;
}

/** Original character illustrations for the landing page's Magic cards. */
const cards: Card[] = [
  {
    name: "Sol Ring",
    cost: "{1}",
    type: "Artifact",
    caption: "A little extra mana.",
    art: String.raw`           . * .
        .         .
      .    _____    .
     .   /#######\   .
     *  /##/   \##\  *
    .   |##|   |##|   .
     *  \##\___/##/  *
     .   \#######/   .
      .    -----    .
        .         .
           ' * '
          __| |__`,
  },
  {
    name: "Lightning Bolt",
    cost: "{R}",
    type: "Instant",
    caption: "Three damage. One mana.",
    art: String.raw`      .--.    .--.
   .-(    '--'   )-.
  (________________)
              /##/
            /###/
          /####/____
         /########/
             /###/
           /###/
          /##/
         /#/
  ______/ /____________`,
  },
  {
    name: "Birds of Paradise",
    cost: "{G}",
    type: "Creature - Bird",
    caption: "A bird. Any color of mana.",
    art: String.raw`           __
       ___(o )>
      /####\_/
     /#####/
    /#####/___
    \###/     \
     \#/       \
  ====||================
      /\       \
     /  \       \
    / /\ \       \
   /_/  \_\  `,
  },
  {
    name: "Counterspell",
    cost: "{U}{U}",
    type: "Instant",
    caption: "Not this time.",
    art: String.raw`   *                 *
         .-----.
       .'       '.
      /    / /    \
  ___|    / /     |___
 / __\   / /     /__ \
 |/  /  / /      \  \|
    /\     /     /\
   /  '.       .'  \
  /     '-----'     \
 /         *        \
        .     .`,
  },
  {
    name: "Swords to Plowshares",
    cost: "{W}",
    type: "Instant",
    caption: "Put the sword to work.",
    art: String.raw`        /\
        ||
        ||        .
        ||       / \
     ===++===   /   \
        ||     /_____\
        ()       | |
                 | |
  _____________ /  |
   ___________ /___/
    ________________
     _______________`,
  },
];

/** Width of a face before it turns, in character cells. */
const faceWidth = 34;
/** Fixed stage width keeps every frame centered without layout changes. */
const stageWidth = 40;
/** One complete hold and front-back-front turn, in milliseconds. */
const cycleDuration = 5580;

/** Pad authored card content without hiding an oversized illustration. */
function row(text = ""): string {
  if (text.length > faceWidth - 2) {
    throw new Error(`Card row is too wide: ${text}`);
  }
  return `|${text.padEnd(faceWidth - 2)}|`;
}

/** Build a full card face with readable name, art, and a short caption. */
function front(card: Card): string[] {
  const art = card.art.split("\n");
  const border = `+${"-".repeat(faceWidth - 2)}+`;
  return [
    border,
    row(` ${card.name}`),
    row(` ${card.cost}`),
    border,
    row(),
    ...art.map((line) => row(`   ${line}`)),
    ...Array.from({ length: 14 - art.length }, () => row()),
    row(),
    border,
    row(` ${card.type}`),
    row(),
    row(` ${card.caption}`),
    row(),
    border,
  ];
}

/** Original hammer-in-a-ring card back shared by every turn. */
const back = front({
  name: "SEIZE THE MANA",
  cost: "[ stm ]",
  type: "CLI + AGENT SKILL",
  caption: "Use what you own.",
  art: String.raw`      #####
    = #######
   ===    ###
  ====      ##
 =====      ##
 ======      ##
  = ====     ##
     ====    ##
      ====  ###
       ==== ##
        ====##
      ###===
    ######=
    ###`,
});

/** Project a card into fewer cells while keeping normal glyph proportions. */
function project(face: string[], width: number): string[] {
  return face.map((line, y) => {
    if (width === 1) return "|";
    if (y === 0 || y === face.length - 1) return `+${"-".repeat(width - 2)}+`;
    if (width === faceWidth) return line;
    const content = Array.from({ length: width - 2 }, (_, x) => {
      const source =
        1 + Math.floor(((x + 0.5) * (faceWidth - 2)) / (width - 2));
      const glyph = line[source];
      return width < 20 && /[a-zA-Z0-9]/.test(glyph) ? ":" : glyph;
    }).join("");
    return `|${content}|`;
  });
}

/** Frame for an elapsed time; face changes only while the card is edge-on. */
export function frameAt(elapsed: number): CardFrame {
  const index = Math.floor(elapsed / cycleDuration) % cards.length;
  const time = elapsed % cycleDuration;
  const next = (index + 1) % cards.length;
  let face = front(cards[index]);
  let width = faceWidth;
  if (time >= 4200 && time < 4440) {
    width = Math.max(
      1,
      Math.round(faceWidth * Math.cos((((time - 4200) / 240) * Math.PI) / 2)),
    );
  } else if (time >= 4440 && time < 4500) {
    width = 1;
  } else if (time >= 4500 && time < 4740) {
    face = back;
    width = Math.max(
      1,
      Math.round(faceWidth * Math.sin((((time - 4500) / 240) * Math.PI) / 2)),
    );
  } else if (time >= 4740 && time < 5040) {
    face = back;
  } else if (time >= 5040 && time < 5280) {
    face = back;
    width = Math.max(
      1,
      Math.round(faceWidth * Math.cos((((time - 5040) / 240) * Math.PI) / 2)),
    );
  } else if (time >= 5280 && time < 5340) {
    width = 1;
  } else if (time >= 5340) {
    face = front(cards[next]);
    width = Math.max(
      1,
      Math.round(faceWidth * Math.sin((((time - 5340) / 240) * Math.PI) / 2)),
    );
  }
  const padding = " ".repeat(Math.floor((stageWidth - width) / 2));
  const lines = project(face, width).map((line) =>
    (padding + line).padEnd(stageWidth),
  );
  const empty = " ".repeat(stageWidth);
  return {
    text: [empty, ...lines, empty].join("\n"),
    delay:
      time < 4200
        ? 4200 - time
        : time >= 4740 && time < 5040
          ? 5040 - time
          : 1000 / 24,
  };
}
