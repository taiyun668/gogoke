import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { InstancesPage, type InstancePageSource } from "@/features/instances/InstancesPage";
import { pageFromDesign37 } from "@/features/instances/instancePageModel";
import { readDesign37InstancesSnapshot } from "./design37Instances";

/**
 * Settings > 实例 on today's host commands: state, login progress and runtime
 * issues, plus login and cancel. The host's register command enrolls only a
 * fixed test instance, so it is not offered as creating an instance.
 */
export function createDesign37InstanceSource(): InstancePageSource {
  const read = async () =>
    pageFromDesign37(readDesign37InstancesSnapshot(await invoke<unknown>("gogoke_design37_instances")));
  const command = async (name: string, instanceId: string) => {
    readDesign37InstancesSnapshot(await invoke<unknown>(name, { instanceId }));
  };
  return {
    read,
    actions: {
      login: (id) => command("gogoke_design37_instance_login", id),
      cancelLogin: (id) => command("gogoke_design37_instance_cancel", id),
    },
  };
}

export function Design37InstanceSection() {
  const source = useMemo(createDesign37InstanceSource, []);
  return <InstancesPage source={source} />;
}
