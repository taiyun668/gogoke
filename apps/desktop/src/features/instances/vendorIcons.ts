import codexIcon from "./icons/ProviderIcon-codex.svg";
import claudeIcon from "./icons/ProviderIcon-claude.svg";
import opencodeIcon from "./icons/ProviderIcon-opencode.svg";
import grokIcon from "./icons/ProviderIcon-grok.svg";
import antigravityIcon from "./icons/ProviderIcon-antigravity.svg";
import type { VendorId } from "./instancePageModel";

/** CodexBar provider marks (MIT, see icons/CODEXBAR-LICENSE.txt), drawn as currentColor masks. */
export const VENDOR_ICONS: Record<VendorId, string> = {
  codex: codexIcon,
  claude: claudeIcon,
  opencode: opencodeIcon,
  grok: grokIcon,
  antigravity: antigravityIcon,
};
