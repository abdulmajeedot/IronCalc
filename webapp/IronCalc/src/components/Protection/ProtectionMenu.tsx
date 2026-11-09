import { Check, EyeOff, Lock, LockOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Menu } from "../Menu/Menu";
import { MenuDivider } from "../Menu/MenuDivider";
import { MenuItem } from "../Menu/MenuItem";

export type ProtectionAction =
  | "protect"
  | "unprotect"
  | "toggle_locked"
  | "toggle_hidden";

export interface ProtectionState {
  isProtected: boolean;
  locked: boolean;
  hidden: boolean;
}

// Every change the protection of a sheet refuses fails with this text.
export function isProtectedSheetError(error: unknown): boolean {
  return `${error}`.includes("This sheet is protected");
}

const ProtectionMenu = (props: {
  anchor: { x: number; y: number };
  state: ProtectionState;
  onClose: () => void;
  onAction: (action: ProtectionAction) => void;
}) => {
  const { anchor, state, onClose, onAction } = props;
  const { t } = useTranslation();
  const act = (action: ProtectionAction) => {
    onClose();
    onAction(action);
  };

  return (
    <Menu open onClose={onClose} anchorPosition={anchor}>
      {state.isProtected ? (
        <MenuItem icon={<LockOpen />} onClick={() => act("unprotect")}>
          {t("protection.unprotect_sheet")}
        </MenuItem>
      ) : (
        <MenuItem icon={<Lock />} onClick={() => act("protect")}>
          {t("protection.protect_sheet")}
        </MenuItem>
      )}
      <MenuDivider />
      <MenuItem
        icon={state.locked ? <Check /> : undefined}
        disabled={state.isProtected}
        onClick={() => act("toggle_locked")}
      >
        {t("protection.lock_cell")}
      </MenuItem>
      <MenuItem
        icon={state.hidden ? <Check /> : <EyeOff />}
        disabled={state.isProtected}
        onClick={() => act("toggle_hidden")}
      >
        {t("protection.hide_formula")}
      </MenuItem>
    </Menu>
  );
};

export default ProtectionMenu;
