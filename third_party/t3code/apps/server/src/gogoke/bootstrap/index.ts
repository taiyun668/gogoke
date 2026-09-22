import {
  constructNativeStoreServiceForAdapter,
  type GogokeNativeStoreConstructionRequest,
  type GogokeNativeStoreService,
} from "./nativeStoreService.ts";
import { RootProfileOwnership } from "./rootProfileOwnership.ts";
import { NativeHostClient } from "../persistence/base/nativeHostClient.ts";

/**
 * Canonical local Gogoke bootstrap entry.
 *
 * The public product entry always composes the typed native store. It never
 * accepts a caller-supplied persistence constructor or SQLite filename.
 */
type GogokeServiceInput = GogokeNativeStoreConstructionRequest & {
  readonly ownership?: RootProfileOwnership;
};

export function constructGogokeService(
  input: GogokeServiceInput,
): Promise<GogokeNativeStoreService> {
  return constructNativeStoreServiceForAdapter({
    request: input.request,
    root: input.root,
    hostBinary: input.hostBinary,
    ownership: input.ownership ?? new RootProfileOwnership(),
    connector: { attach: (request) => NativeHostClient.attach(request) },
  });
}

export type {
  GogokeNativeStoreConstructionRequest,
  GogokeNativeStoreService,
} from "./nativeStoreService.ts";
export type { MinimalServiceConstructionRequest } from "./minimalService.ts";
export { RootProfileOwnership, RootProfileOwnershipError } from "./rootProfileOwnership.ts";
