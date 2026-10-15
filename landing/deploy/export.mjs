











import { spawn } from "node:child_process";
import { cp, mkdir, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const deployDir = dirname(fileURLToPath(import.meta.url));
const landingDir = join(deployDir, "..");
const profile = process.argv.includes("--release") ? "release" : "debug";
const publicDir = join(deployDir, "public");
const bundleDir = join(landingDir, "target", profile, "assets");
const binary = join(landingDir, "target", profile, "smbl-dainiki-landing");
const port = process.env.EXPORT_PORT ?? "3971";
const base = `http://127.0.0.1:${port}`;

const PLACEHOLDER = "<!--WAITLIST_STATUS-->";

const HEADERS = `# Long-lived caching for the content-hashed bundle and images.
/*.css
  Cache-Control: public, max-age=31536000, immutable
/*.png
  Cache-Control: public, max-age=31536000, immutable
/*.jpg
  Cache-Control: public, max-age=31536000, immutable

/index.html
  Cache-Control: public, max-age=0, must-revalidate
`;

async function waitForServer(timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(base);
      if (response.ok) return;
    } catch {
      
    }
    await delay(250);
  }
  throw new Error(`app did not come up on ${base}`);
}

const app = spawn(binary, [], {
  cwd: landingDir,
  env: { ...process.env, PORT: port },
  stdio: "ignore",
});

try {
  await waitForServer();
  const html = await (await fetch(base)).text();

  
  
  const formEnd = html.indexOf("</form>");
  if (formEnd === -1) throw new Error("could not find the waitlist form");
  const page = [
    html.slice(0, formEnd + "</form>".length),
    "\n",
    PLACEHOLDER,
    html.slice(formEnd + "</form>".length),
  ].join("");

  await rm(publicDir, { recursive: true, force: true });
  await mkdir(publicDir, { recursive: true });
  await writeFile(join(publicDir, "index.html"), page);
  await writeFile(join(publicDir, "_headers"), HEADERS);

  for (const entry of await readdir(bundleDir)) {
    if (entry === "manifest.toml") continue; 
    await cp(join(bundleDir, entry), join(publicDir, entry));
  }

  
  
  await cp(join(landingDir, "src/landing/img/og.png"), join(publicDir, "og.png"));

  
  let rewritten = page;
  for (const entry of await readdir(publicDir)) {
    rewritten = rewritten.replaceAll(`/_topcoat/assets/${entry}`, `/${entry}`);
  }
  await writeFile(join(publicDir, "index.html"), rewritten);

  const files = (await readdir(publicDir)).join(", ");
  console.log(`exported to deploy/public (${profile} binary): ${files}`);
  if (rewritten.includes("/_topcoat/assets/")) {
    throw new Error("unrewritten asset URLs remain in index.html");
  }
} finally {
  app.kill();
}
