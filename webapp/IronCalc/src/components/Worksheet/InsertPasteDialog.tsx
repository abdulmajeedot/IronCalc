import { useState } from "react";
import { useTranslation } from "react-i18next";
import { DialogShell } from "../RightDrawer/PivotTable/PivotMenus";

export const InsertPasteDialog = (props: {
  // Wide copies push cells down and tall ones push them right, like Excel
  defaultDown: boolean;
  onClose: () => void;
  onConfirm: (down: boolean) => void;
}) => {
  const { t } = useTranslation();
  const [down, setDown] = useState(props.defaultDown);
  const option = (isDown: boolean, label: string) => (
    <label className="ic-insert-paste-option">
      <input
        type="radio"
        name="ic-insert-paste"
        checked={down === isDown}
        onChange={() => setDown(isDown)}
      />
      {label}
    </label>
  );
  return (
    <DialogShell
      title={t("insert_paste.title")}
      onClose={props.onClose}
      onConfirm={() => props.onConfirm(down)}
    >
      <div className="ic-pivot-label">{t("insert_paste.insert")}</div>
      {option(false, t("context_menu.cell.shift_cells_right"))}
      {option(true, t("context_menu.cell.shift_cells_down"))}
    </DialogShell>
  );
};
