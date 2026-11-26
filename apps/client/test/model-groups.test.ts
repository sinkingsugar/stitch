import { expect, test } from "bun:test";
import { modelGroups, modelName } from "../src/transcript/modelGroups";

test("providers and their models keep node order and full IDs", () => {
  expect([...modelGroups(["openrouter/a/b", "anthropic/opus", "openrouter/c/d", "local"])])
    .toEqual([["openrouter", ["openrouter/a/b", "openrouter/c/d"]], ["anthropic", ["anthropic/opus"]], ["local", ["local"]]]);
  expect(modelName("openrouter/a/b")).toBe("a/b");
  expect(modelName("local")).toBe("local");
});
