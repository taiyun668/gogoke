import * as NodeAssert from "node:assert/strict";
import * as NodeTest from "node:test";

import { EpochSingleflight, HostCoordinatorError } from "./coordinator.ts";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

test("same epoch singleflights 32 callers onto one construct", async () => {
  const flight = new EpochSingleflight<string>();
  let constructs = 0;
  const callers = Array.from({ length: 32 }, () =>
    flight.join("connect", "1", async () => {
      constructs += 1;
      return "ready";
    }),
  );
  const results = await Promise.all(callers);
  assert.equal(constructs, 1);
  assert.equal(
    results.every((value) => value === "ready"),
    true,
  );
});

test("a newer epoch cannot clear a still-current older connection", async () => {
  const flight = new EpochSingleflight<string>();
  let release!: (value: string) => void;
  const held = new Promise<string>((resolve) => {
    release = resolve;
  });
  const first = flight.join("connect", "1", () => held);
  assert.throws(
    () => {
      void flight.join("connect", "2", async () => "second");
    },
    (error: unknown) => error instanceof HostCoordinatorError && error.code === "FAILED_CUSTODY",
  );
  release("first");
  assert.equal(await first, "first");
});
