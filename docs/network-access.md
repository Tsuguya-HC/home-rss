# 外部アクセスと CiliumNetworkPolicy

このリポジトリのコンポーネントが**どこに出られるか**は、ここではなく
[home-cluster](https://github.com/Tsuguya-HC/home-cluster) の `manifests/rss/netpol-rss-apps.yaml`
が決める。`spin.toml` の `allowed_outbound_hosts` を開けても、CNP が閉じていれば通信は通らない。

**塞がれた egress は失敗ではなくハングする。** SYN が黙って落ちるので、アプリ側からは
「応答が返らない」としか見えず、タイムアウトまで待たされる。原因がコードに見えるので、
この層を疑うまでに時間を溶かしやすい。

## 現状（2026-09-15）

| コンポーネント | 出られる先 |
|---|---|
| `rss-server` | `rss-pg:5432` / **`world:443`** |
| `rss-fetcher` | `rss-pg:5432` / **`world:443`** |
| `rss-ui` | `rss-pg:5432` |
| `rss-cleaner` | `rss-pg:5432` |
| `rss-cron` | `rss-fetcher:80` / `rss-cleaner:80` / `kube-apiserver:6443` / `seaweedfs filer:8333` |
| `rss-workflow-exit`（Argo の exit hook） | `kube-apiserver:6443` / `seaweedfs filer:8333` / `discord.com:443` |
| `rss-migration` | `rss-pg:5432` |

DNS は CCNP（`allow-dns`）でクラスタ全 Pod に共通適用されているので、個別の CNP には書かない。

### `world` が意味する範囲

Cilium の `world` は **「クラスタ外」全般**で、インターネットだけでなく**同じ LAN 上の
非クラスタ機器**（NAS・ルータ・UniFi・他の内部サービスの 443）も含む。一方、クラスタ内部の
サービスは identity ベースの default-deny で別途遮断される。

つまり `world:443` を開けたコンポーネントは、**ユーザーが指定した URL で家庭 LAN 上の機器に
到達できる**。`rss-server` の `POST /api/feeds`（即時取得）がこれに当たるため、アプリ側でも
`shared/src/ssrf.rs` の `reject_internal_feed_url()` で内部宛を弾いている。ただし DNS
リバインディングと、内部ホストを指す外部 DNS 名は防げない — **最終的な境界はネットワーク側**。

## 新しい外部アクセスが要るとき

1. **PR の概要に明示する。** 「このコンポーネントが新しくどこへ出る必要があるか」を書く
2. `spin.toml` の `allowed_outbound_hosts` を足す
3. **home-cluster 側の CNP もセットで足す。** 片方だけでは動かない
4. 動かないときは、まず**ハングしているのか失敗しているのか**を見る。ハングなら CNP を疑う

このリポジトリで作業するエージェント（実装・レビュー）は home-rss しか clone しないので、
**CNP の不足は構造的に見えない**。「クラスタ側の手当てが要る」と旗を立てるところまでが
このリポジトリの担当で、実際に足すのは home-cluster 側の仕事になる。

なお記事のサムネイル画像（`image_url`）は一覧表示時に閲覧者のブラウザが直接取得するもので、
`rss-server` / `rss-fetcher` の egress ではない。画像が表示されなくても CNP や
`allowed_outbound_hosts` を開ける必要はない（#145）。

## 過去に踏んだ例

- 2026-09-15: `POST /api/feeds` の即時取得を足したが、`rss-server` の CNP に `world:443` が無く、
  **フィード追加のたびに待たされて「取得失敗」**になる状態でマージ寸前まで進んだ
  （home-cluster #931 で修正）。`rss-fetcher` は元から同じルールを持っていて、
  **server が外に出る想定が無かっただけ**
