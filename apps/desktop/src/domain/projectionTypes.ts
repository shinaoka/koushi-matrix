import type { AvatarImage, MentionTarget } from "./types";

export type MentionCandidate = {
  key: string;
  label: string;
  searchText: string;
  avatar?: AvatarImage | null;
  target: MentionTarget;
};

export type TimelineForwardDestination = {
  room_id: string;
  display_name: string;
};
