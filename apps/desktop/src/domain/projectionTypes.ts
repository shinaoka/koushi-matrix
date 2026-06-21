import type { MentionTarget } from "./types";

export type MentionCandidate = {
  key: string;
  label: string;
  searchText: string;
  target: MentionTarget;
};

export type TimelineForwardDestination = {
  room_id: string;
  display_name: string;
};
