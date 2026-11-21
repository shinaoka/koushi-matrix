import { t } from "../../i18n/messages";
import { failureKindLabel } from "./SettingsStatusPrimitives";
import type { RoomKeyExportState, RoomKeyImportState } from "../../domain/types";

/**
 * Key-management status rows, shared by the Security section and the migration
 * surface so one command's state reads the same in both places.
 */
export function KeyManagementStatus({
  label,
  value,
  testId
}: {
  label: string;
  value: string;
  testId: string;
}) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small data-testid={testId}>{value}</small>
    </div>
  );
}

export function roomKeyExportStatusLabel(status: RoomKeyExportState): string {
  switch (status.kind) {
    case "idle":
      return t("settings.roomKeyExportIdle");
    case "exporting":
      return t("settings.roomKeyExporting");
    case "exported":
      return status.exported_sessions === null
        ? t("settings.roomKeyExportedUnknown")
        : t("settings.roomKeyExportedCount", { count: status.exported_sessions });
    case "failed":
      return t("settings.roomKeyExportFailed", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

export function roomKeyImportStatusLabel(status: RoomKeyImportState): string {
  switch (status.kind) {
    case "idle":
      return t("settings.roomKeyImportIdle");
    case "importing":
      return t("settings.roomKeyImporting");
    case "imported":
      return t("settings.roomKeyImportedCount", {
        imported: status.imported_count,
        total: status.total_count
      });
    case "failed":
      return t("settings.roomKeyImportFailed", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}
