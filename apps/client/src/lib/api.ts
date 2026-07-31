import { invoke } from "@tauri-apps/api/core";

export interface OperatorInfo {
  id: string;
  name: string;
}

export interface LoginResponse {
  token: string;
  expires_at: string;
  device_id: string;
  operator: OperatorInfo;
}

/** Invokes the `login` Tauri command (src-tauri/src/commands/auth.rs), which
 * calls the Go backend's `POST /auth/login`. */
export async function login(
  deviceId: string,
  operatorPin: string,
): Promise<LoginResponse> {
  return invoke<LoginResponse>("login", { deviceId, operatorPin });
}
