import type { EncoderGymBridge } from "../preload.js";

declare global {
  interface Window {
    encoderGym: EncoderGymBridge;
  }
}

export {};
