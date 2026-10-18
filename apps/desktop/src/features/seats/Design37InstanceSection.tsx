import { useMemo } from "react";
import { InstancesPage, type InstancePageSource } from "@/features/instances/InstancesPage";
import { createManagedCliLifecycleSource } from "@/features/instances/managedCliLifecycleSource";

/**
 * Settings > 实例 uses the managed instance bridge and the native CLI lifecycle
 * status. The page only offers operations backed by the host.
 */
export function createDesign37InstanceSource(): InstancePageSource {
  return createManagedCliLifecycleSource();
}

export function Design37InstanceSection() {
  const source = useMemo(createDesign37InstanceSource, []);
  return <InstancesPage source={source} />;
}
