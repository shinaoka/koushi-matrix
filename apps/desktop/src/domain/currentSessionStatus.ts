import type {
  CurrentSessionStatusDetails,
  CurrentSessionStatusState
} from "./types";

export function currentSessionStatusFactsRemainAuthoritative(
  status: CurrentSessionStatusState
): boolean {
  return (
    status.status !== "failed" ||
    status.kind === "timed_out" ||
    status.kind === "connectivity_unavailable" ||
    status.kind === "network"
  );
}

export function currentSessionStatusDetails(
  status: CurrentSessionStatusState
): CurrentSessionStatusDetails | null {
  switch (status.status) {
    case "ready":
      return status.details;
    case "checking":
    case "failed":
      return status.last_known_details;
    case "idle":
      return null;
  }
}
