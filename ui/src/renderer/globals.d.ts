export interface EncoderGymBridge {
  windowAction(action: "minimize" | "maximize" | "close"): Promise<void>;
  copyText(value: string): Promise<void>;
  versions(): { electron: string; chrome: string; node: string };
}

declare global {
  interface Window {
    encoderGym: EncoderGymBridge;
  }
}

export {};
