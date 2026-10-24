export interface UpdateInfo {
  current_version: string;
  latest_version: string;
  available: boolean;
  release_url: string;
  notes: string;
  asset_name: string | null;
  asset_size: number | null;
}

export interface DownloadedUpdate {
  path: string;
  version: string;
}

export interface DownloadProgress {
  downloaded: number;
  total: number;
}
