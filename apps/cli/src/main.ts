import { runCli } from "./commands.js";

runCli(process.argv.slice(2)).then((code) => {
  process.exitCode = code;
});
