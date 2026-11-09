import type { ProtectionPermissions } from "@ironcalc/wasm";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "../Checkbox/Checkbox";
import { Input } from "../Input/Input";
import { DialogShell } from "../RightDrawer/PivotTable/PivotMenus";
import "./protection.css";

const SHOWN_PERMISSIONS: (keyof ProtectionPermissions)[] = [
  "select_locked_cells",
  "select_unlocked_cells",
  "format_cells",
  "format_columns",
  "format_rows",
  "insert_columns",
  "insert_rows",
  "insert_hyperlinks",
  "delete_columns",
  "delete_rows",
  "sort",
  "auto_filter",
  "pivot_tables",
  "objects",
];

const DEFAULT_PERMISSIONS: ProtectionPermissions = {
  select_locked_cells: true,
  select_unlocked_cells: true,
  format_cells: false,
  format_columns: false,
  format_rows: false,
  insert_columns: false,
  insert_rows: false,
  insert_hyperlinks: false,
  delete_columns: false,
  delete_rows: false,
  sort: false,
  auto_filter: false,
  pivot_tables: false,
  objects: false,
  scenarios: false,
};

export const ProtectSheetDialog = (props: {
  onClose: () => void;
  onProtect: (
    password: string | null,
    permissions: ProtectionPermissions,
  ) => void;
}) => {
  const { onClose, onProtect } = props;
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [repeated, setRepeated] = useState("");
  const [permissions, setPermissions] = useState(DEFAULT_PERMISSIONS);
  const mismatch = password !== repeated;

  return (
    <DialogShell
      title={t("protection.dialog_title")}
      onClose={onClose}
      confirmDisabled={mismatch}
      onConfirm={() => {
        if (!mismatch) {
          onProtect(password === "" ? null : password, permissions);
        }
      }}
    >
      <div className="ic-protect-sheet">
        <Input
          type="password"
          autoComplete="new-password"
          label={t("protection.password")}
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <Input
          type="password"
          autoComplete="new-password"
          label={t("protection.repeat_password")}
          value={repeated}
          error={mismatch && repeated !== ""}
          helperText={
            mismatch && repeated !== ""
              ? t("protection.password_mismatch")
              : undefined
          }
          onChange={(event) => setRepeated(event.target.value)}
        />
        <p className="ic-protect-sheet-note">{t("protection.password_note")}</p>
        <div className="ic-protect-sheet-title">
          {t("protection.allow_title")}
        </div>
        <div className="ic-protect-sheet-list">
          {SHOWN_PERMISSIONS.map((name) => (
            <Checkbox
              key={name}
              checked={permissions[name]}
              label={t(`protection.permission_${name}`)}
              onChange={(checked) =>
                setPermissions({ ...permissions, [name]: checked })
              }
            />
          ))}
        </div>
      </div>
    </DialogShell>
  );
};
