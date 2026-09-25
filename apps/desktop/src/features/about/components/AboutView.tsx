import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppLanguagePreference } from "@/types";
import { getAppSettings } from "@services/tauri";
import { I18nProvider, useI18n } from "@/i18n";

const GITHUB_URL = "https://github.com/taiyun668/gogoke";

export function AboutView() {
  const [language, setLanguage] = useState<AppLanguagePreference>("system");

  useEffect(() => {
    let active = true;
    void getAppSettings()
      .then((settings) => {
        if (active) {
          setLanguage(settings.appLanguage);
        }
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  return (
    <I18nProvider language={language}>
      <AboutViewContent />
    </I18nProvider>
  );
}

function AboutViewContent() {
  const { tx } = useI18n();
  const [version, setVersion] = useState<string | null>(null);

  const handleOpenGitHub = () => {
    void openUrl(GITHUB_URL);
  };

  useEffect(() => {
    let active = true;
    const fetchVersion = async () => {
      try {
        const value = await invoke<string>("gogoke_product_version");
        if (active) {
          setVersion(value);
        }
      } catch {
        if (active) {
          setVersion(null);
        }
      }
    };

    void fetchVersion();
    return () => {
      active = false;
    };
  }, []);

  return (
    <div className="about">
      <div className="about-card">
        <div className="about-header">
          <img
            className="about-icon"
            src="/app-icon.png"
            alt={tx("gogoke icon")}
          />
          <div className="about-title">{tx("gogoke")}</div>
        </div>
        <div className="about-version">
          {version ? tx("Version {version}", { version }) : tx("Version —")}
        </div>
        <div className="about-tagline">
          {tx("Monitor the situation of your Codex agents")}
        </div>
        <div className="about-divider" />
        <div className="about-links">
          <button
            type="button"
            className="about-link"
            onClick={handleOpenGitHub}
          >
            GitHub
          </button>
        </div>
        <div className="about-footer">{tx("Built for gogoke")}</div>
      </div>
    </div>
  );
}
