import { useEffect, useState } from "react";

import { dismissNotice, listDismissedNotices } from "../../lib/ipc";
import { Banner, type BannerProps } from "./Banner";

interface NoticeBannerProps extends Omit<BannerProps, "onDismiss"> {
  /** Persistent id: once dismissed the banner never shows again, across restarts. */
  id: string;
}

export function NoticeBanner({ id, ...banner }: NoticeBannerProps) {
  // `null` until the dismissed list arrives, so a dismissed banner never flashes.
  const [visible, setVisible] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    listDismissedNotices()
      .then((ids) => {
        if (!cancelled) setVisible(!ids.includes(id));
      })
      .catch(() => {
        if (!cancelled) setVisible(true);
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  if (!visible) return null;

  const dismiss = () => {
    setVisible(false);
    // Best effort: a failed write only brings the banner back next session.
    dismissNotice(id).catch(() => {});
  };

  return <Banner {...banner} onDismiss={dismiss} />;
}
