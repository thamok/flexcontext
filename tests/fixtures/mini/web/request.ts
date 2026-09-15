import { runtimeConfig } from "./config";

export interface ParsedRequest {
  requestId: string;
  bearerToken?: string;
}

/** Convert inbound headers into the request type used by handlers. */
export function parseRequest(headers: Record<string, string>): ParsedRequest {
  const prefix = runtimeConfig.authenticationPrefix;
  return {
    requestId: headers["x-request-id"],
    bearerToken: headers.authorization?.replace(prefix, ""),
  };
}
