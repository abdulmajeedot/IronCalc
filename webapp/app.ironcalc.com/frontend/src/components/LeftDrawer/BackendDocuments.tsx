import { Button, Confirm, IconButton, Input } from "@ironcalc/workbook";
import { Link2, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type BackendDocument,
  clearToken,
  type DocumentSnapshot,
  deleteDocument,
  getSelectedDocumentId,
  getToken,
  InvalidTokenError,
  isSelectedOnBackend,
  isSelectedPaused,
  listDocuments,
  setToken,
} from "../documents";
import ReferenceDialog from "./ReferenceDialog";

const REFRESH_MS = 30_000;

interface BackendDocumentsProps {
  onOpen: (document: BackendDocument) => Promise<void>;
  onUpload: () => Promise<void>;
  onInsertReference: (
    document: BackendDocument,
    snapshot: DocumentSnapshot,
    sheet: string,
    range: string,
  ) => string | null;
}

function BackendDocuments({
  onOpen,
  onUpload,
  onInsertReference,
}: BackendDocumentsProps) {
  const { t } = useTranslation();
  const [connected, setConnected] = useState(() => getToken() !== null);
  const [tokenInput, setTokenInput] = useState("");
  const [documents, setDocuments] = useState<BackendDocument[]>([]);
  const [error, setError] = useState("");
  const [toDelete, setToDelete] = useState<BackendDocument | null>(null);
  const [referenceTo, setReferenceTo] = useState<BackendDocument | null>(null);

  const report = useCallback(
    (e: unknown) => {
      if (e instanceof InvalidTokenError) {
        clearToken();
        setConnected(false);
        setDocuments([]);
        setError(t("left_drawer.backend.invalid_token"));
      } else {
        setError(t("left_drawer.backend.unreachable"));
      }
    },
    [t],
  );

  const refresh = useCallback(async () => {
    try {
      setDocuments(await listDocuments());
      setError("");
    } catch (e) {
      report(e);
    }
  }, [report]);

  useEffect(() => {
    if (!connected) {
      return;
    }
    refresh();
    const interval = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(interval);
  }, [connected, refresh]);

  const run = async (action: () => Promise<void>) => {
    try {
      await action();
      await refresh();
    } catch (e) {
      report(e);
    }
  };

  if (!connected) {
    return (
      <div className="app-ic-backend">
        <div className="app-ic-backend-title">
          {t("left_drawer.backend.title")}
        </div>
        <form
          className="app-ic-backend-connect"
          onSubmit={(event) => {
            event.preventDefault();
            if (tokenInput.trim() === "") {
              return;
            }
            setToken(tokenInput.trim());
            setTokenInput("");
            setError("");
            setConnected(true);
          }}
        >
          <Input
            type="password"
            autoComplete="off"
            aria-label={t("left_drawer.backend.token")}
            placeholder={t("left_drawer.backend.token")}
            value={tokenInput}
            onChange={(event) => setTokenInput(event.target.value)}
          />
          <Button type="submit" variant="secondary" size="sm">
            {t("left_drawer.backend.connect")}
          </Button>
        </form>
        {error ? <div className="app-ic-backend-error">{error}</div> : null}
      </div>
    );
  }

  return (
    <div className="app-ic-backend">
      <div className="app-ic-backend-title">
        {t("left_drawer.backend.title")}
      </div>
      <div className="app-ic-backend-list">
        {documents.length === 0 ? (
          <div className="app-ic-backend-empty">
            {t("left_drawer.backend.empty")}
          </div>
        ) : (
          documents.map((document) => (
            <div className="app-ic-backend-row" key={document.id}>
              <button
                type="button"
                className="app-ic-backend-name"
                title={document.name}
                onClick={() => run(() => onOpen(document))}
              >
                {document.name}
              </button>
              {document.id === getSelectedDocumentId() ? null : (
                <IconButton
                  icon={<Link2 />}
                  aria-label={t("left_drawer.backend.reference")}
                  title={t("left_drawer.backend.reference")}
                  onClick={() => setReferenceTo(document)}
                />
              )}
              <IconButton
                icon={<Trash2 />}
                aria-label={t("left_drawer.backend.delete")}
                onClick={() => setToDelete(document)}
              />
            </div>
          ))
        )}
      </div>
      {error ? <div className="app-ic-backend-error">{error}</div> : null}
      {isSelectedPaused() ? (
        <div className="app-ic-backend-error">
          {t("left_drawer.backend.paused")}
        </div>
      ) : null}
      <div className="app-ic-backend-actions">
        {!isSelectedOnBackend() || isSelectedPaused() ? (
          <Button variant="secondary" size="sm" onClick={() => run(onUpload)}>
            {t("left_drawer.backend.upload")}
          </Button>
        ) : null}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            clearToken();
            setConnected(false);
            setDocuments([]);
            setError("");
          }}
        >
          {t("left_drawer.backend.disconnect")}
        </Button>
      </div>
      {referenceTo ? (
        <ReferenceDialog
          document={referenceTo}
          onClose={() => setReferenceTo(null)}
          onInsert={(snapshot, sheet, range) => {
            const failure = onInsertReference(
              referenceTo,
              snapshot,
              sheet,
              range,
            );
            if (failure) {
              return t(`left_drawer.backend.reference_${failure}`);
            }
            setReferenceTo(null);
            return null;
          }}
        />
      ) : null}
      <Confirm
        open={toDelete !== null}
        onClose={() => setToDelete(null)}
        onConfirm={() => {
          const target = toDelete;
          setToDelete(null);
          if (target) {
            run(() => deleteDocument(target.id));
          }
        }}
        variant="destructive"
        title={t("left_drawer.backend.delete_title")}
        message={t("left_drawer.backend.delete_message", {
          name: toDelete?.name ?? "",
        })}
        confirmLabel={t("left_drawer.backend.delete")}
        cancelLabel={t("left_drawer.backend.cancel")}
        closeLabel={t("left_drawer.backend.cancel")}
      />
    </div>
  );
}

export default BackendDocuments;
