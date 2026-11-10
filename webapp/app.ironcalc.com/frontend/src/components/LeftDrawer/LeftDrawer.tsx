import { useState } from "react";
import type { BackendDocument, DocumentSnapshot } from "../documents";
import DrawerContent from "./DrawerContent";
import DrawerFooter from "./DrawerFooter";
import DrawerHeader from "./DrawerHeader";
import { useWorkbookSelection } from "./useWorkbookSelection";
import "./left-drawer.css";

interface LeftDrawerProps {
  open: boolean;
  newModel: () => void;
  setModel: (key: string) => void;
  onDelete: (uuid: string) => void;
  onOpenDocument: (document: BackendDocument) => Promise<void>;
  onUploadDocument: () => Promise<void>;
  onInsertReference: (
    document: BackendDocument,
    snapshot: DocumentSnapshot,
    sheet: string,
    range: string,
  ) => string | null;
}

function LeftDrawer({
  open,
  newModel,
  setModel,
  onDelete,
  onOpenDocument,
  onUploadDocument,
  onInsertReference,
}: LeftDrawerProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const {
    checkedUuids,
    handleCheckboxClick,
    handleDeleteChecked,
    handleCancelChecked,
  } = useWorkbookSelection({ onDelete });

  return (
    <div className={`app-ic-drawer${open ? " app-ic-drawer--open" : ""}`}>
      <div className="app-ic-drawer-paper">
        <DrawerHeader
          onNewModel={newModel}
          searchQuery={searchQuery}
          setSearchQuery={setSearchQuery}
        />
        <DrawerContent
          setModel={setModel}
          onDelete={onDelete}
          onOpenDocument={onOpenDocument}
          onUploadDocument={onUploadDocument}
          onInsertReference={onInsertReference}
          searchQuery={searchQuery}
          checkedUuids={checkedUuids}
          onCheckboxClick={handleCheckboxClick}
        />
        <DrawerFooter
          checkedCount={checkedUuids.size}
          onDeleteChecked={handleDeleteChecked}
          onCancelChecked={handleCancelChecked}
        />
      </div>
    </div>
  );
}

export default LeftDrawer;
