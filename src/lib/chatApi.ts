import { invoke } from "@tauri-apps/api/core";

/** See `src-tauri/src/chat.rs`. */
export interface Conversation {
  id: number;
  title: string;
  /** null = whatever Claude Code picks on its own. */
  model: string | null;
  /** null = the model's default effort. */
  effort: Effort | null;
  createdAt: number;
  updatedAt: number;
}

/** What `claude --effort` accepts, lowest first. */
export const EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const;
export type Effort = (typeof EFFORTS)[number];

export type Part =
  | { type: "text"; text: string }
  | {
      type: "tool";
      id: string;
      /** Without the `mcp__otomo__` prefix. */
      name: string;
      input: Record<string, unknown>;
      status: "running" | "ok" | "error";
      result: string | null;
    };

export interface Message {
  id: number;
  role: "user" | "assistant";
  parts: Part[];
  createdAt: number;
  model: string | null;
  /** The level asked for; null when the model's default was used. */
  effort: Effort | null;
  costUsd: number | null;
  durationMs: number | null;
  error: string | null;
  stopped: boolean;
}

export interface ChatStatus {
  claudePath: string | null;
  serverPath: string | null;
  blocked: string | null;
}

/** `chat://tool`: a call starting, its input arriving, or its result. */
export interface ToolEvent {
  convId: number;
  id: string;
  name: string | null;
  input: Record<string, unknown> | null;
  status: "running" | "ok" | "error" | null;
  result: string | null;
}

export const chatStatus = () => invoke<ChatStatus>("chat_status");
export const listConversations = () => invoke<Conversation[]>("chat_list");
export const listMessages = (convId: number) => invoke<Message[]>("chat_messages", { convId });
export const runningConversations = () => invoke<number[]>("chat_running");
export const stopChat = (convId: number) => invoke<void>("chat_stop", { convId });
export const deleteConversation = (convId: number) => invoke<void>("chat_delete", { convId });
export const setConversationModel = (convId: number, model: string | null) =>
  invoke<void>("chat_set_model", { convId, model });
export const setConversationEffort = (convId: number, effort: Effort | null) =>
  invoke<void>("chat_set_effort", { convId, effort });

export const sendChat = (args: {
  convId: number | null;
  text: string;
  song: { id: number; title: string } | null;
  model: string | null;
  effort: Effort | null;
}) => invoke<Conversation>("chat_send", args);
