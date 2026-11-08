import { Button, Model } from "@ironcalc/workbook";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLanguageFromLocale,
  loadDefaultLocaleFromStorage,
} from "../storage";
import { useSelectedUuid } from "../useStorage";
import { aiLog } from "./aiLog";
import {
  getVersionBytes,
  listVersions,
  type WorkbookVersion,
} from "./versions";

export function VersionsPanel(properties: {
  onOpenCopy: (model: Model) => void;
}) {
  const { onOpenCopy } = properties;
  const { t } = useTranslation();
  const uuid = useSelectedUuid();
  const [versions, setVersions] = useState<WorkbookVersion[]>([]);

  useEffect(() => {
    let isCurrent = true;
    setVersions([]);
    if (uuid) {
      listVersions(uuid)
        .then((list) => {
          if (isCurrent) {
            setVersions(list);
          }
        })
        .catch((e) => aiLog("warn", "version_list_failed", { error: e }));
    }
    return () => {
      isCurrent = false;
    };
  }, [uuid]);

  const openCopy = async (version: WorkbookVersion) => {
    try {
      const bytes = await getVersionBytes(version.id);
      if (bytes) {
        const language = getLanguageFromLocale(loadDefaultLocaleFromStorage());
        const model = Model.fromBytes(bytes, language);
        model.setName(`${version.name} (${t("ai_sidebar.restored")})`);
        onOpenCopy(model);
      }
    } catch (e) {
      aiLog("error", "version_restore_failed", { error: e });
    }
  };

  return (
    <div className="app-ic-ai-sidebar-panel">
      <div className="app-ic-ai-sidebar-panel-title">
        {t("ai_sidebar.versions")}
      </div>
      {versions.length === 0 && (
        <div className="app-ic-ai-sidebar-hint">
          {t("ai_sidebar.no_versions")}
        </div>
      )}
      {versions.map((version) => (
        <div key={version.id} className="app-ic-ai-sidebar-version">
          <div className="app-ic-ai-sidebar-version-text">
            <div>{version.request}</div>
            <div className="app-ic-ai-sidebar-hint">
              {new Date(version.createdAt).toLocaleString()}
            </div>
          </div>
          <Button variant="outline" size="xs" onClick={() => openCopy(version)}>
            {t("ai_sidebar.open_copy")}
          </Button>
        </div>
      ))}
    </div>
  );
}
