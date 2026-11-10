import type { BackendDocument, DocumentSnapshot } from "../documents";
import BackendDocuments from "./BackendDocuments";
import LocalStorageAlert from "./LocalStorageAlert";
import WorkbookList from "./WorkbookList";

interface DrawerContentProps {
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
  searchQuery: string;
  checkedUuids: Set<string>;
  onCheckboxClick: (
    uuid: string,
    shiftKey: boolean,
    orderedUuids: string[],
  ) => void;
}

function DrawerContent({
  setModel,
  onDelete,
  onOpenDocument,
  onUploadDocument,
  onInsertReference,
  searchQuery,
  checkedUuids,
  onCheckboxClick,
}: DrawerContentProps) {
  return (
    <>
      <div className="app-ic-drawer-content">
        <WorkbookList
          setModel={setModel}
          onDelete={onDelete}
          searchQuery={searchQuery}
          checkedUuids={checkedUuids}
          onCheckboxClick={onCheckboxClick}
        />
        <BackendDocuments
          onOpen={onOpenDocument}
          onUpload={onUploadDocument}
          onInsertReference={onInsertReference}
        />
      </div>
      <div
        className={`app-ic-drawer-alert-wrapper${checkedUuids.size > 0 ? " app-ic-drawer-alert-wrapper--selection" : ""}`}
      >
        <LocalStorageAlert />
      </div>
    </>
  );
}

export default DrawerContent;
