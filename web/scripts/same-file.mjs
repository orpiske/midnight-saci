// Fails when the committed generated file differs from a fresh generation,
// so docs/openapi.yaml and src/api/schema.d.ts cannot drift apart.
import { readFileSync, rmSync } from "node:fs";

const [committed, fresh] = process.argv.slice(2);
const same = readFileSync(committed, "utf8") === readFileSync(fresh, "utf8");
rmSync(fresh);
if (!same) {
  console.error(`${committed} is out of date with docs/openapi.yaml; run \`npm run api-types\`.`);
  process.exit(1);
}
