import { createContext, useContext, useRef, type KeyboardEvent, type ReactNode } from "react";
import { useI18n } from "@/i18n";
import Folder from "lucide-react/dist/esm/icons/folder";
import GitBranch from "lucide-react/dist/esm/icons/git-branch";
import ScrollText from "lucide-react/dist/esm/icons/scroll-text";
import Users from "lucide-react/dist/esm/icons/users";
import MessagesSquare from "lucide-react/dist/esm/icons/messages-square";

export type LegacyPanelTabId = "git" | "files" | "prompts";
export type PanelTabId = LegacyPanelTabId | "seats" | "sidechat";
export type PanelTabsSelection = { active: PanelTabId; onSelect: (id: PanelTabId) => void };
const Selection = createContext<PanelTabsSelection | null>(null);
/** The integrator owns extended selection; old panels retain their three-tab props. */
export function PanelTabsScope({ value, children }: { value: PanelTabsSelection; children: ReactNode }) {
  return <Selection.Provider value={value}>{children}</Selection.Provider>;
}

type PanelTab = {
  id: PanelTabId;
  label: string;
  icon: ReactNode;
};

type PanelTabsProps = {
  active: PanelTabId;
  onSelect: (id: LegacyPanelTabId) => void;
  tabs?: PanelTab[];
};

const defaultTabs: PanelTab[] = [
  { id: "git", label: "Git", icon: <GitBranch aria-hidden /> },
  { id: "files", label: "Files", icon: <Folder aria-hidden /> },
  { id: "prompts", label: "Prompts", icon: <ScrollText aria-hidden /> },
];

export function PanelTabs({ active, onSelect, tabs = defaultTabs }: PanelTabsProps) {
  const selection = useContext(Selection);
  if (selection) {
    tabs = [...defaultTabs,
      { id: "seats", label: "席位", icon: <Users aria-hidden /> },
      { id: "sidechat", label: "旁聊", icon: <MessagesSquare aria-hidden /> },
    ];
  }
  const selected = selection?.active ?? active;
  const pick = (id: PanelTabId) => {
    if (selection) selection.onSelect(id);
    else if (id === "git" || id === "files" || id === "prompts") onSelect(id);
  };
  const { tx } = useI18n();
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const activeIndex = tabs.findIndex((tab) => tab.id === selected);
  const focusableIndex = activeIndex >= 0 ? activeIndex : 0;

  const selectByIndex = (index: number, options?: { focus?: boolean }) => {
    if (tabs.length === 0) {
      return;
    }
    const normalized = (index + tabs.length) % tabs.length;
    pick(tabs[normalized].id);
    if (options?.focus) {
      tabRefs.current[normalized]?.focus();
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (tabs.length <= 1) {
      return;
    }
    const currentIndex = activeIndex >= 0 ? activeIndex : index;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      selectByIndex(currentIndex + 1, { focus: true });
      return;
    }
    if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      selectByIndex(currentIndex - 1, { focus: true });
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      selectByIndex(0, { focus: true });
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      selectByIndex(tabs.length - 1, { focus: true });
    }
  };

  return (
    <div className="panel-tabs" role="tablist" aria-label={tx("Panel")} aria-orientation="horizontal">
      {tabs.map((tab, index) => {
        const isActive = selected === tab.id;
        const label = tx(tab.label);
        return (
          <button
            key={tab.id}
            type="button"
            className={`panel-tab${isActive ? " is-active" : ""}`}
            onClick={() => pick(tab.id)}
            onKeyDown={(event) => handleKeyDown(event, index)}
            ref={(element) => {
              tabRefs.current[index] = element;
            }}
            role="tab"
            aria-selected={isActive}
            tabIndex={index === focusableIndex ? 0 : -1}
            aria-label={label}
            title={label}
          >
            <span className="panel-tab-icon" aria-hidden>
              {tab.icon}
            </span>
          </button>
        );
      })}
    </div>
  );
}
