const fs = require("fs");

function random(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const [input, output, seed = "7"] = process.argv.slice(2);
const next = random(Number(seed));
const between = (low, high) => Math.round(low + next() * (high - low));

const lines = fs.readFileSync(input, "utf8").split("\n").flatMap((line) => {
  const match = line.match(/^Type~ "(.*)"$/);
  if (!match) return [line];
  const out = [];
  const text = match[1];
  for (let index = 0; index < text.length; ) {
    const size = between(1, 3);
    out.push(`Type@${between(25, 80)}ms "${text.slice(index, index + size)}"`);
    index += size;
    if (next() < 0.15) out.push(`Sleep ${between(80, 220)}ms`);
  }
  return out;
});

fs.writeFileSync(output, lines.join("\n"));
