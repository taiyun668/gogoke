import { useMemo } from "react";
import { InstancesPage, type InstancePageSource } from "@/features/instances/InstancesPage";
import { pageFromManaged } from "@/features/instances/instancePageModel";
import { createDesign37ManagedInstanceSource } from "@/services/design37ManagedInstances";

/**
 * Settings > 实例 on the host's managed instance bridge. Every field and action
 * comes from the bridge; operations it does not implement (check, upgrade,
 * rollback, remove) are absent, so the page does not offer them.
 */
export function createDesign37InstanceSource(): InstancePageSource {
  return createDesign37ManagedInstanceSource(pageFromManaged);
}

export function Design37InstanceSection() {
  const source = useMemo(createDesign37InstanceSource, []);
  return <InstancesPage source={source} />;
}
