import { describe, expect, it } from "vitest";
import { createPreviewHost } from "./host";
import { readDesign37InstancesSnapshot } from "../design37Instances";

describe("browser preview through the existing K-UI forwarding fake", () => {
  it("keeps the original pending request across reads and cancellation, then allows a fresh login", async () => {
    const host = createPreviewHost();
    const args = { instanceId: "codexTestM1" };
    const firstRaw = await host.invoke("gogoke_design37_instance_login", args);
    const first = readDesign37InstancesSnapshot(firstRaw);
    expect(first.instances[0].login?.state).toBe("PENDING");
    const same = await host.invoke("gogoke_design37_instances");
    expect(same).toEqual(firstRaw);
    const duplicate = await host.invoke("gogoke_design37_instance_login", args);
    expect(duplicate).toEqual(firstRaw);
    const cancelled = readDesign37InstancesSnapshot(await host.invoke("gogoke_design37_instance_cancel", args));
    expect(cancelled.instances[0].login?.state).toBe("CANCELLED");
    const fresh = readDesign37InstancesSnapshot(await host.invoke("gogoke_design37_instance_login", args));
    expect(fresh.instances[0].login?.state).toBe("PENDING");
    host.settle(false);
    const failed = readDesign37InstancesSnapshot(await host.invoke("gogoke_design37_instances"));
    expect(failed.instances[0].state).toBe("ERROR");
    expect(failed.instances[0].login?.error).toBe("PREVIEW_CLI_FAILED: synthetic failure");
    await host.invoke("gogoke_design37_instance_login", args);
    host.settle(true);
    const done = await host.invoke("gogoke_design37_instances");
    expect(readDesign37InstancesSnapshot(done).instances[0].state).toBe("LOGGED_IN");
    await host.invoke("gogoke_design37_instance_cancel", args);
    expect(await host.invoke("gogoke_design37_instances")).toEqual(done);
  });

  it("rejects native commands and other instance identities", async () => {
    const host = createPreviewHost();
    await expect(host.invoke("gogoke_design37_user_operation")).rejects.toThrow("PREVIEW_UNSUPPORTED_COMMAND");
    await expect(host.invoke("gogoke_design37_instance_login", { instanceId: "actualOtherInstance" })).rejects.toThrow("PREVIEW_INSTANCE_MISMATCH");
  });
});
