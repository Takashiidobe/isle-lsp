const { chromium } = require("playwright-core");
const path = require("path");

const SIZE = { width: 1400, height: 800 };

const cursor = () => {
  const arrow =
    "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><path d='M3 2 L3 19 L7.5 14.8 L10.6 21.5 L13.2 20.3 L10.1 13.7 L16.5 13.7 Z' fill='black' stroke='white' stroke-width='1.5' stroke-linejoin='round'/></svg>";
  const pointer = document.createElement("div");
  pointer.style.cssText = `position:fixed;z-index:99999;width:24px;height:24px;pointer-events:none;left:-40px;top:-40px;background:url("data:image/svg+xml,${encodeURIComponent(arrow)}") no-repeat`;
  document.addEventListener("DOMContentLoaded", () => document.body.appendChild(pointer));
  document.addEventListener("mousemove", (e) => {
    pointer.style.left = e.clientX - 3 + "px";
    pointer.style.top = e.clientY - 2 + "px";
  });
};

let state = 7;
function next() {
  state = (state + 0x6d2b79f5) >>> 0;
  let t = state;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const between = (low, high) => Math.round(low + next() * (high - low));

let pointer = { x: 300, y: 120 };

async function moveTo(page, x, y) {
  const steps = between(14, 30);
  for (let step = 1; step <= steps; step++) {
    const eased = 0.5 - Math.cos((step / steps) * Math.PI) / 2;
    await page.mouse.move(
      pointer.x + (x - pointer.x) * eased + (step < steps ? between(-2, 2) : 0),
      pointer.y + (y - pointer.y) * eased + (step < steps ? between(-2, 2) : 0),
    );
    await page.waitForTimeout(between(8, 20));
  }
  pointer = { x, y };
}

async function hoverAll(page, selector, { dx = 60, limit = Infinity } = {}) {
  const items = (await page.locator(selector).all()).slice(0, limit);
  for (const item of items) {
    await item.scrollIntoViewIfNeeded();
    const box = await item.boundingBox();
    await moveTo(page, box.x + Math.min(dx, box.width - 10) + between(-20, 20), box.y + box.height * (0.3 + next() * 0.4));
    await page.waitForTimeout(between(250, 550));
  }
}

async function toggleAll(page, selector, { limit = Infinity } = {}) {
  const folds = (await page.locator(selector).all()).slice(0, limit);
  for (const fold of folds) {
    const box = await fold.boundingBox();
    await moveTo(page, box.x + 40, box.y + box.height / 2);
    await page.waitForTimeout(between(300, 500));
    await fold.click();
    await page.waitForTimeout(between(500, 800));
    await fold.click();
    await page.waitForTimeout(between(400, 700));
  }
}

const scenarios = {
  async trie(page) {
    await page.waitForTimeout(between(900, 1400));
    await hoverAll(page, ".n.leaf", { dx: 120, limit: 6 });
    await toggleAll(page, "summary.n.arm", { limit: 2 });
    await page.waitForTimeout(between(600, 1000));
  },

  async rust(page) {
    await page.waitForTimeout(between(900, 1400));
    await hoverAll(page, ".code .rule", { dx: 160 });
    await page.waitForTimeout(between(600, 1000));
  },

  async graph(page) {
    await page.waitForTimeout(between(900, 1400));
    await hoverAll(page, ".term", { dx: 60 });
    await page.waitForTimeout(between(600, 1000));
  },
};

async function main() {
  const [name, htmlFile, videoFile] = process.argv.slice(2);
  const scenario = scenarios[name];
  if (!scenario) throw new Error(`no scenario for ${name}`);
  const browser = await chromium.launch({ executablePath: "/usr/bin/chromium", headless: true });
  const context = await browser.newContext({
    viewport: SIZE,
    colorScheme: "dark",
    recordVideo: { dir: path.dirname(videoFile), size: SIZE },
  });
  await context.addInitScript(cursor);
  const page = await context.newPage();
  await page.goto("file://" + path.resolve(htmlFile));
  await scenario(page);
  const video = page.video();
  await context.close();
  await video.saveAs(videoFile);
  await video.delete();
  await browser.close();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
