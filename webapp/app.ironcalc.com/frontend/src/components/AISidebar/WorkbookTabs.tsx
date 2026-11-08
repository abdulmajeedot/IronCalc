import { FileSpreadsheet, X } from "lucide-react";
import { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { useModelsMetadata, useSelectedUuid } from "../useStorage";
import {
  closeWorkbookTab,
  getOpenTabs,
  openWorkbookTab,
  subscribeToTabs,
} from "./openTabs";

import "./workbook-tabs.css";

/** Open workbooks, shown once there is more than one to switch between. */
export function WorkbookTabs(properties: { onSelect: (uuid: string) => void }) {
  const { onSelect } = properties;
  const { t } = useTranslation();
  const metadata = useModelsMetadata();
  const selected = useSelectedUuid();
  const opened = useSyncExternalStore(subscribeToTabs, getOpenTabs);

  useEffect(() => {
    if (selected) {
      openWorkbookTab(selected);
    }
  }, [selected]);

  const tabs = opened.filter((uuid) => metadata[uuid]);
  if (tabs.length < 2) {
    return null;
  }

  const close = (uuid: string) => {
    if (uuid === selected) {
      const position = tabs.indexOf(uuid);
      onSelect(tabs[position + 1] ?? tabs[position - 1]);
    }
    closeWorkbookTab(uuid);
  };

  return (
    <div className="app-ic-workbook-tabs">
      {tabs.map((uuid) => (
        <div
          key={uuid}
          className={`app-ic-workbook-tab${uuid === selected ? " app-ic-workbook-tab--active" : ""}`}
        >
          <button
            type="button"
            className="app-ic-workbook-tab-name"
            aria-current={uuid === selected}
            onClick={() => uuid !== selected && onSelect(uuid)}
          >
            <FileSpreadsheet aria-hidden="true" />
            <span>{metadata[uuid].name}</span>
          </button>
          <button
            type="button"
            className="app-ic-workbook-tab-close"
            aria-label={t("ai_sidebar.close_tab", {
              name: metadata[uuid].name,
            })}
            onClick={() => close(uuid)}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}
