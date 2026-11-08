import { mergeConfig } from "vite";
import base from "./vite.config.ts";

// Local development only: sends /api calls to the Rocket server, so the app
// works from the Vite URL without running Caddy.
export default mergeConfig(base, {
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8000",
    },
  },
});
