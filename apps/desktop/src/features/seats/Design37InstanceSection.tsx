import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { InstancesPage, type InstancePageSource } from "@/features/instances/InstancesPage";
import { pageFromDesign37 } from "@/features/instances/instancePageModel";
import { DESIGN37_TEST_INSTANCE_ID, readDesign37InstancesSnapshot } from "./design37Instances";

/**
 * Settings > 实例 on today's host commands. The host reports state, version,
 * verified newer version, login progress and runtime issues, and accepts
 * register, login and cancel; the page shows only controls the host can serve.
 */
export function createDesign37InstanceSource(): InstancePageSource {
  const read = async () =>
    pageFromDesign37(readDesign37InstancesSnapshot(await invoke<unknown>("gogoke_design37_instances")));
  const command = async (name: string, instanceId: string) => {
    readDesign37InstancesSnapshot(await invoke<unknown>(name, { instanceId }));
  };
  return {
    read,
    createTakesName: false,
    // Today the host registers only the fixed Codex test instance.
    canCreate: (section) => section.vendor === "codex" && section.instances.length === 0,
    actions: {
      login: (id) => command("gogoke_design37_instance_login", id),
      cancelLogin: (id) => command("gogoke_design37_instance_cancel", id),
      create: async (vendor) => {
        if (vendor !== "codex") throw new Error("宿主目前只能登记 Codex 测试实例");
        await command("gogoke_design37_instance_register", DESIGN37_TEST_INSTANCE_ID);
        await command("gogoke_design37_instance_login", DESIGN37_TEST_INSTANCE_ID);
      },
    },
  };
}

export function Design37InstanceSection() {
  const source = useMemo(createDesign37InstanceSource, []);
  return <InstancesPage source={source} />;
}
