/**
 * TypeScript SDK for the nexode harness API.
 *
 * ```ts
 * import { NexodeClient } from "@nexode/agent-sdk";
 * const client = await NexodeClient.connect({ clientName: "my-app/1.0" });
 * const session = await client.createSession(process.cwd());
 * const turn = await client.run(session.session_id, "hello");
 * console.log(turn.text);
 * client.close();
 * ```
 */

export * from "./protocol.js";
export * from "./sockets.js";
export * from "./framing.js";
export { HarnessError } from "./errors.js";
export {
  launchInstance,
  inheritCredentials,
  userNexodeHome,
  userAppConfigDir,
} from "./launch.js";
export type { LaunchOptions, LaunchedInstance } from "./launch.js";
export { bundledNexodeBinary, platformBinaryPackage } from "./binary.js";
export { NexodeClient, unixSocketTransport } from "./client.js";
export type { SdkTool, SessionToolsOptions, ToolExecutionContext, ToolResult } from "./tools.js";
export type {
  AssistantTextMessage,
  ConnectOptions,
  CreateSessionOptions,
  FileContent,
  FileStatus,
  GlobalEventsOptions,
  RunOptions,
  RunStructuredOptions,
  RuntimeInfo,
  SendMessageOptions,
  StructuredTurnResult,
  Transport,
  TurnResult,
} from "./client.js";
export { StructuredOutputError } from "./structured.js";
export type {
  StructuredOutputAttempt,
  StructuredOutputSchema,
  StructuredValidationIssue,
} from "./structured.js";
