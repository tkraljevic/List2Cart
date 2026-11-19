import iconUrl from "../assets/app-icon.png";
import type { AboutViewState } from "./contracts";

/**
 * The App Store and Google Play limit links to outside payments, donations included,
 * so the Ko-fi link is shown only in the desktop app.
 */
const onPhoneOrTablet = typeof navigator !== "undefined" && /Android|iPhone|iPad|iPod/i.test(navigator.userAgent);

export const appMetadata: AboutViewState = {
  name: "List2Cart",
  version: __APP_VERSION__,
  license: __APP_LICENSE__,
  iconUrl,
  supportUrl: onPhoneOrTablet ? null : "https://ko-fi.com/tkraljevic",
};
