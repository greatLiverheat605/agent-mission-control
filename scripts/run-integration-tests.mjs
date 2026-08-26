import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const args = process.argv.slice(2);
let pattern;
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "--grep") pattern = args[++index];
  else if (args[index].startsWith("--grep=")) pattern = args[index].slice(7);
  else throw new Error(`unsupported integration test argument: ${args[index]}`);
}
if (pattern === undefined && args.length > 0) throw new Error("--grep requires a value");

const directory = join(process.cwd(), "tests", "integration");
const files = readdirSync(directory)
  .filter((file) => file.endsWith(".test.mjs"))
  .sort()
  .map((file) => join(directory, file));
const nodeArgs = ["--test"];
if (pattern) nodeArgs.push(`--test-name-pattern=${pattern}`);
nodeArgs.push(...files);

const result = spawnSync(process.execPath, nodeArgs, { stdio: "inherit" });
process.exit(result.status ?? 1);
