# home-rss の仕様

今の main の動きと、変えるときに壊してはいけない性質。振る舞いを変えたらここも直す。

## 1. データと、その書き手・読み手

### テーブルと削除の連鎖

| テーブル | 主な制約 |
|---|---|
| `feeds` | `id` UUID 主キー、`url` NOT NULL UNIQUE（素のテキスト比較） |
| `articles` | `id` UUID 主キー、`feed_id` → `feeds(id)` **ON DELETE CASCADE**、UNIQUE(`feed_id`, `url`)、`title` NOT NULL |
| `read_status` | `article_id` 主キー → `articles(id)` **ON DELETE CASCADE**。行があれば既読、無ければ未読 |

- 削除は `feeds` → `articles` → `read_status` の順に連鎖する。フィードを消すとその記事と既読が、記事を消すとその既読が消える
- `articles.feed_id` は NULL を許すが、NULL を書く経路は無い
- 時刻列（`created_at` / `fetched_at` / `read_at`）はどれも DB の既定値 `now()` で入り、アプリは値を渡さない。`last_fetched_at` だけはアプリが `NOW()` で書く
- `read_status.read_at` を読む経路は無い
- 既読・未読は `read_status` の行の有無だけで決まる。未読を表す列は無く、既読を取り消す経路も無い

### 経路ごとの読み書き

R = 読む、W = 書く（INSERT / UPDATE）、D = 消す。

| 経路 | `feeds` | `articles` | `read_status` |
|---|---|---|---|
| `GET /api/feeds` | R（`created_at` 降順） | | |
| `POST /api/feeds` | W: `INSERT … ON CONFLICT (url) DO UPDATE` で既存なら既存行を返す → 即時取得で W | 即時取得で W | |
| `DELETE /api/feeds/{id}` | D | 連鎖で D | 連鎖で D |
| `GET /api/articles?feed_id=&unread=` | | R | R（`unread=true` のとき） |
| `POST /api/articles/{id}/read` | | （外部キーで参照） | W: `ON CONFLICT DO NOTHING` |
| `POST /api/articles/read-all` | | R（全フィード） | W: 未読の全記事ぶん |
| `POST /api/import/opml` | W: `INSERT … ON CONFLICT (url) DO NOTHING`。取得はしない | | |
| `GET /api/stats` | R（件数） | R | R |
| fetcher（`/fetch`） | R（`id, url, etag, last_modified` と失敗判定用の `last_fetched_at` の全行）→ フィードごとに W | W | |
| cleaner（`/clean`） | | D（既読かつ古いもの） | 連鎖で D |

即時取得と fetcher の取得は同じ `shared/src/fetch.rs` の `fetch_only()` が行い、保存は同じ `store()` を `store_fetched()` / `fetch_and_store()` 経由で呼ぶ。

- 取得前に `reject_internal_feed_url()` を毎回通す。弾かれた定期取得は失敗として記録する（下の fetcher の失敗記録を参照）。即時取得は 400 で行を作らない
- 条件付き GET（`If-None-Match` / `If-Modified-Since`）。**200 のときだけ**記事と `title` 等を書く。304 は記事も `last_fetched_at` も触らず、失敗の記録だけを消す。失敗（`FetchFailed` / 解釈不能）は記事も `last_fetched_at` も触らず、失敗の記録だけを書く。保存の失敗（`StoreFailed`）は失敗の記録に触れない
- 200 のとき、`articles` に全エントリを UNNEST の 1 文で `INSERT … ON CONFLICT DO NOTHING`。既存の記事は更新しない（タイトル・本文・`image_url` が後から変わっても反映されない）
- 続けて `feeds` の `title` / `site_url` / `etag` / `last_modified` を今回の応答とフィードの値で**上書き**し（無ければ NULL）、`last_fetched_at = NOW()`。即時取得は `etag` / `last_modified` を渡さない（条件付き GET にしない）

列ごとの書き手:

- `feeds.url`: 追加と OPML インポートだけ。保存するのは `reject_internal_feed_url()` が返した正規化後の URL
- `feeds` のそれ以外: 200 のときの `store()` だけ（即時取得は `store_fetched()` 経由、fetcher は `fetch_and_store()` 経由）。ただし `last_fetch_error` / `fetch_failing_since` は `store()` が書かず、fetcher の `process_feed` が取得結果に応じた独立の UPDATE として書く・消す（即時取得は書きも消しもしない）
- `articles`: `store()` の INSERT だけ（経路は同上）。UPDATE する経路は無い
- `read_status`: 既読 API と全既読 API だけ

### UI の画面操作と API

| 操作 | 呼ぶ API | 補足 |
|---|---|---|
| 画面を開く | `GET /api/feeds`、`GET /api/articles?unread=true`、`GET /api/articles`（選択中の条件） | サイドバーの未読数は未読記事一覧を UI 側で数えたもの |
| フィード選択・「未読のみ」切り替え | `GET /api/articles` | 「未読のみ」の初期値は ON |
| 記事を開く | `POST /api/articles/{id}/read` | 既読の記事でも開くたびに呼ぶ。「未読のみ」表示中は開いた記事を一覧から外す |
| 全既読 | `POST /api/articles/read-all` | フィードを選択していても全フィードが既読になる。UI の未読数は全部 0 にする |
| フィード追加 | `POST /api/feeds`（45 秒で中断） | 成否に関わらず、続けてフィード一覧と記事一覧を読み直す |
| フィード削除 | `DELETE /api/feeds/{id}` | 3 秒以内の 2 回目のクリックで実行。後でフィード一覧と記事一覧を読み直す |
| OPML インポート | `POST /api/import/opml` | フィード一覧だけ読み直す。不正・拒否でスキップしたものがあると、件数の内訳をエラー表示に出す |

- `GET /api/stats` は UI から呼ばれていない
- 追加以外の API は 30 秒で中断する
- 記事本文は DOMPurify でサニタイズして表示する。サムネイルは `referrerPolicy="no-referrer"` で閲覧者のブラウザが直接取得し、読み込みに失敗したら隠す

## 2. 定期処理と同時実行

### fetcher

- Spin の **http trigger**（route `/fetch`、メソッドは問わない）。1 リクエストで 1 回分の取得をする
- 誰がいつ呼ぶか、前の回が終わる前に次を呼ぶかは**リポの外で決まる**
- 1 回分の動き: `feeds` を 1 度だけ全件読み、1 件ずつ順に `fetch_and_store()`（タイムアウト無し）。1 件の失敗はログに出して次へ進み、応答は 200。DB 接続と最初の SELECT の失敗だけが 500
- 失敗の記録（定期取得だけ）: `FetchFailed`（送信・受信のエラー、URL ガードでの拒否、リクエストの組み立て失敗、200 / 304 以外の応答。3xx も含む）と解釈不能な本文は、一度でも取得できたフィードだけ `last_fetch_error`（直近の理由、200 文字で切る）と `fetch_failing_since`（連続失敗の始まり。失敗が続く間は上書きしない）に書く。200 と 304 は両列を NULL に戻す（304 は `last_fetched_at` を更新しない）。`StoreFailed` と即時取得（`POST /api/feeds`）は両列に触れない。一度も取得していないフィード（OPML 直後など）には印を付けない。応答しないフィードがその回で記録されないのは範囲外。判定は `fetcher/src/failure.rs` の純粋関数に切り出す
- 読んだ後に追加されたフィードは、その回には取得されない

### cleaner

- Spin の **http trigger**（route `/clean`）。呼ぶ周期は**リポの外で決まる**
- 1 文の `DELETE`: `read_status` に行があり、かつ `fetched_at` が `retention_days` 日より前の記事。未読は消さない。基準は `fetched_at` で、`published_at` や `read_at` ではない
- `retention_days` は Spin 変数（既定 30）。読めない・数値でないときも 30

### server の即時取得

- `POST /api/feeds` の中で、追加（または既存）の 1 フィードだけを取得する。取得は `fetch_only()` でトランザクションの外、保存は `store_fetched()` で `shared::tx::in_transaction` の 1 トランザクションの中。取得中は行ロックを取らないので fetcher・DELETE・同時追加を待たせない
- 送信と本文読み取りのそれぞれを 15 秒で打ち切る（WASI の outbound HTTP にタイムアウトが無いため）
- 応答: 取得して保存できたら 201 と取得後の行、取得失敗 502、パース不能 422、保存失敗 500、JSON 不正と URL の拒否は 400（DB に触れない）
- 取得・パースの失敗ではトランザクションを始めず、新規の行は残らない。保存（INSERT と記事・`feeds` の書き込み）の失敗では新規の行は ROLLBACK され、何も残らない。既存の行への再追加は保存の失敗でも COMMIT して残し、行を変えない

### トランザクション

- `POST /api/feeds` の保存（INSERT と `store_fetched()` の書き込み）だけが `shared::tx::in_transaction` で BEGIN〜COMMIT/ROLLBACK する。他の SQL 文は単独で自動コミットされる
- 接続は保存の直前（取得の後）にリクエストごとに `db::connect()` で開く。取得の失敗では接続もトランザクションも作らない
- `fetch_and_store()` の記事 INSERT と `feeds` UPDATE は別の文で、その間に他の経路が入りうる。記事 INSERT は 1 文なので、1 回分の記事は全部入るか全部入らないか
- OPML インポートは URL ごとに別の INSERT。途中で DB エラーになると、それまでの行は残って 500 を返す

### 同じテーブルを同時に触る組み合わせ

| 組み合わせ | 起きること |
|---|---|
| 同じフィードの fetcher と即時取得（または fetcher 2 つ） | 記事は UNIQUE(`feed_id`, `url`) と `ON CONFLICT DO NOTHING` で重複しない。`feeds` の `title` / `etag` などは後から UPDATE した方が残る。失敗の記録と即時取得の保存は別の列なので干渉しない（失敗の記録は fetcher だけが書く） |
| 取得中にそのフィードを `DELETE` | 即時取得は取得中に行ロックを取らないので、取得中の DELETE は待たずに実行される。DELETE が INSERT より先なら INSERT が新規行を作り直して保存は成功し 201 を返す（残骸になる。再追加の取得は終わっているので取り直さない）。INSERT と保存の書き込みは 1 トランザクションで行ロックを保持するので、その間に来た DELETE は COMMIT/ROLLBACK まで待ってから実行されるだけ |
| cleaner が消した記事がまだフィードに載っている | 次に 200 が返ると UNIQUE に当たらないので、**未読の新しい行として入り直す**（304 なら入らない） |
| cleaner / フィード削除で消えた記事を UI で開く | 既読 API が外部キー違反で 500。UI は再読み込みまで消えた記事を表示し続け、開くたびに既読 API を呼ぶ |
| 全既読と fetcher / 即時取得 | 全既読の文より後に入った記事は未読のまま残る。UI は未読数を 0 にするので、次の読み直しまで表示とずれる |
| OPML インポートと fetcher | インポートした行は、次の fetcher の回で初めて取得される。直後から失敗しても `last_fetched_at` が無い間は失敗の記録は付かない |

## 3. 守るべき性質

テストの場所は、Rust の単体テストと `shared/tests/` の統合テストがファイル名、e2e が `e2e/tests/api.rs`、UI が `*.test.ts(x)`。

### URL の受け入れ（SSRF）

- フィード URL は https・ポート 443・内部ホストでないものだけ受け入れる。localhost / `*.localhost` / `*.internal` / `*.local`（末尾ドット付きを含む）、ループバック・プライベート・リンクローカル・未指定の IPv4、IPv6 のループバック・ULA・リンクローカル、それらを埋め込んだ IPv4-mapped / compat を拒否する — `shared/src/ssrf.rs`: `accepts_https_url_with_public_host`, `rejects_non_https_scheme`, `rejects_non_443_port`, `rejects_localhost`, `rejects_trailing_dot_fqdn_bypass`, `rejects_loopback_ip_literal`, `rejects_unspecified_ip_literal`, `rejects_private_and_link_local_ip_literals`, `rejects_ipv6_ula_and_link_local`, `rejects_ipv4_mapped_and_compat_ipv6_literals`, `rejects_userinfo_bypass`, `rejects_internal_dns_suffixes`
- `POST /api/feeds` は拒否した URL と不正な JSON に 400 を返し、`feeds` に行を作らない — e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`
- ガードは追加時だけでなく `fetch_and_store()` の入口で毎回かかる（fetcher も OPML 由来の行も） — テスト無し
- 保存・取得に使う URL はガードが返した正規化後のもの（スキームとホストは小文字、前後の空白は落とす、危険な文字はエスケープ） — `shared/src/ssrf.rs`: `normalizes_returned_url_scheme_and_host_case`, `trims_leading_and_trailing_whitespace_from_returned_url`, `percent_encodes_unsafe_characters_in_returned_url`（正規化まで。正規化後を保存していることはテスト無し）
- 外に出られるのは、server と fetcher が https（443）と DB、cleaner が DB だけ（各 spin.toml の `allowed_outbound_hosts` が絞る）— テスト無し

### フィードの追加（#106）

- 追加するとその場でそのフィードだけ取得し、成功なら 201 と取得後の行（タイトル等が入ったもの）を返す — `server/src/lib.rs`: `fetched_feed_returns_created`, `fetched_response_body_reflects_updated_feed`（応答への写像だけ。取得から応答までの通しはテスト無し）
- 取得失敗とパース不能は 201 にしない。保存失敗は 502 ではなく 500 — `server/src/lib.rs`: `fetch_failure_is_surfaced_not_created`, `unparseable_feed_is_surfaced_not_created`, `store_failure_is_surfaced_as_server_error_not_bad_gateway`（502 / 422 という値そのものはテスト無し）
- 新規の追加が失敗したら `feeds` の行も記事も残さない — e2e: `adding_a_feed_whose_first_fetch_fails_leaves_no_feed_behind`。失敗が新規行だけを取り消す条件分岐は `server/src/lib.rs`: `only_failed_adds_of_new_feeds_roll_back`
- 既存のフィードへの再追加が失敗しても、その行を消したり変えたりしない — e2e: `readding_an_existing_feed_that_fails_to_fetch_keeps_it_unchanged`（`SET title = NULL` への変異で落ちることを確認済み）
- 同じ URL を再度追加しても行は増えず、既存行を取得し直して返す — テスト無し
- 即時取得は送信と本文の読み取りをそれぞれ 15 秒で打ち切り（合計で最大約 30 秒）、定期取得は打ち切らない — `shared/src/fetch.rs`: `returns_some_when_future_resolves_before_timeout`, `returns_none_when_timeout_resolves_first`, `none_timeout_returns_the_future_result_without_racing`
- UI の追加の待ち時間（45 秒）は、即時取得の打ち切り 2 回分と 10 秒の余裕以上 — `server/src/lib.rs`: `fetch_timeout_is_positive_and_matches_ui_expectation`（UI 側の 45 はテストの中の定数）、`ui/src/api.test.ts`: `gives adding a feed 45 seconds before timing out`（`ui/src/api.ts` の値を経由する）

### 取得と保存

- 200 だけを取得成功とし、304 は変更無し、それ以外は失敗 — `shared/src/fetch.rs`: `classifies_ok_not_modified_and_unexpected`
- 304 と `FetchFailed` / 解釈不能の失敗では記事と `last_fetched_at` に触れない（失敗の記録だけが別の UPDATE で動く。304 は記録を消し、失敗は記録を書く） — 304 での `last_fetched_at` 不変はコード目視（`process_feed` と `clear_fetch_failure` の UPDATE 文に `last_fetched_at` が無いこと）。振り分けは `fetcher/src/failure.rs`: `fetch_failed_is_recorded`, `unparseable_body_is_recorded`, `stored_feed_clears_the_record`, `not_modified_clears_the_record`, `store_failure_leaves_the_record_alone`, `failure_before_any_successful_fetch_leaves_no_mark`, `failure_reason_is_truncated_to_200_chars`。切り詰めの境界は `fetcher/src/failure.rs`: `keeps_short_reasons_untouched`, `cuts_exactly_at_the_character_limit`, `never_splits_a_multibyte_character`。保存の失敗と即時取得が記録に触れないことはコード目視と e2e: `readding_an_existing_feed_that_fails_to_fetch_keeps_it_unchanged`
- 既存の記事は上書きしない。同じフィード・同じ URL の記事は 1 行だけ — テスト無し
- 定期取得の失敗は `feeds` に記録され、成功で消える — e2e: `fetcher_records_a_guard_rejection_as_a_fetch_failure`（失敗の記録と `fetch_failing_since` の不変）、e2e: `feed_list_exposes_the_fetch_failure_record`（JSON への露出）。理由の文面と 200 文字制限は `fetcher/src/failure.rs`: `fetch_failed_is_recorded`, `failure_reason_is_truncated_to_200_chars`
- 失敗中のフィードはサイドバーの名前の横に警告マーク（⚠。削除確認の `!` とは別）を出し、title に理由と失敗し始めた時刻（ローカル時刻）を入れる — `ui/src/components/FeedItem.test.tsx`: `shows a warning for a failing feed`, `shows no warning for a healthy feed`（時刻の値そのものはブラウザでの目視）
- 1 フィードの失敗で fetcher の他のフィードを止めない — テスト無し
- パースできない本文はエラーにする — `shared/src/feed.rs`: `unparseable_body_is_an_error`
- フィードのタイトル・サイト URL、記事の URL・タイトル・本文・公開日時を取り出す。本文は content が無ければ summary — `shared/src/feed.rs`: `parses_feed_title_site_url_and_entry`
- 公開日時は published が無ければ updated — テスト無し
- リンクの無いエントリは捨てる。タイトルの無いエントリは `(no title)` — `shared/src/feed.rs`: `entry_without_link_is_skipped`, `entry_without_title_gets_default`
- 記事 URL は rel=alternate を最優先し、無ければ enclosure でも self でもないもの、次に enclosure、最後に先頭のリンク。1 本しかなければそれを使う（エントリを落とさない）— `shared/src/feed.rs`: `prefers_alternate_link_over_self_link`, `enclosure_is_not_mistaken_for_article_url`, `enclosure_wins_when_only_enclosure_and_self_remain`, `enclosure_wins_when_self_comes_first`, `self_only_entries_keep_distinct_enclosure_urls`, `single_link_entries_are_kept`

### サムネイル（#145）

- 候補の優先順は media:thumbnail → 画像の media:content（RSS の enclosure を含む）→ Atom の画像 enclosure → 本文の最初の `<img>` — `shared/src/feed.rs`: `thumbnail_wins_when_all_candidates_present`, `media_content_wins_over_enclosure`, `enclosure_wins_over_body_img`, `media_content_wins_over_body_img`, `extracts_image_from_rss_enclosure`, `extracts_image_from_media_thumbnail`, `extracts_image_from_media_content`, `extracts_image_from_atom_enclosure`, `extracts_first_img_from_body_when_no_media_or_enclosure`; `shared/tests/feed_image_regressions.rs`: `thumbnail_wins_over_every_other_candidate`, `atom_image_enclosure`
- 画像でないメディアは採らない。type の無い media:content は拡張子（svg を除く）で判定し、type の無い enclosure は採らない。MIME は大文字小文字と前後の空白を無視する — `shared/src/feed.rs`: `non_image_enclosure_is_ignored`, `media_content_with_video_type_is_ignored`, `enclosure_without_type_is_ignored`, `enclosure_without_type_or_length_is_ignored`, `media_content_without_type_is_treated_as_image`, `media_content_without_type_but_with_filesize_is_adopted`, `typeless_image_url_with_query_is_adopted`, `typeless_uppercase_extension_is_adopted`, `typeless_extensionless_url_is_not_adopted`, `typeless_media_content_extension_coverage`, `typeless_filename_equal_to_extension_name_is_not_adopted`, `media_content_with_uppercase_image_type_is_adopted`, `enclosure_media_type_is_case_insensitive`, `media_content_type_with_surrounding_whitespace_is_adopted`, `atom_enclosure_type_with_surrounding_whitespace_is_adopted`, `url_path_has_image_extension_handles_relative_urls`; `shared/tests/feed_image_regressions.rs`: `audio_enclosure_without_type_is_not_an_image`, `media_content_without_type_is_an_image`, `video_media_content_is_not_an_image`
- 保存する画像 URL は記事 URL を基準に絶対化した http / https だけで、2048 文字以下。`#` だけの src は捨てる。無効な候補は飛ばして次を見る — `shared/src/feed.rs`: `dangerous_schemes_are_rejected`, `relative_img_src_is_resolved_against_entry_url`, `overlong_image_url_is_rejected_at_boundary`, `fragment_only_img_src_is_rejected`, `invalid_thumbnail_falls_through_to_enclosure`, `img_without_src_is_skipped_for_next_img`, `body_img_falls_through_invalid_first_src`, `media_content_without_url_does_not_panic`; `shared/tests/feed_image_regressions.rs`: `data_url_pixel_before_real_image_is_skipped`, `invalid_top_candidate_falls_through`
- 本文の `<img>` 探しは HTML コメントの中を見ず、引用符の種類・属性順・大文字小文字・マルチバイト文字に左右されない — `shared/src/feed.rs`: `img_scanner_handles_quote_variants_case_and_attr_order`, `img_in_html_comment_is_ignored`, `img_with_form_feed_separator_is_recognized`, `multibyte_body_img_is_extracted`; `shared/tests/feed_image_regressions.rs`: `image_inside_html_comment_is_ignored`, `multibyte_text_before_image`
- 画像の無い記事は `image_url` が NULL — `shared/src/feed.rs`: `entry_without_image_has_none`; `shared/tests/feed_image_regressions.rs`: `entry_without_any_image`
- 画像の無い記事は一覧でサムネイルの枠を出さず、読み込みに失敗した画像は隠す — `ui/src/components/ArticleListItem.test.tsx`: `renders no thumbnail when image_url is missing`, `hides the thumbnail when the image fails to load`
- 既存の記事の `image_url` は埋め直さない（取得し直しても INSERT が当たらない） — テスト無し

### 記事と既読

- 記事一覧は `feed_id` と `unread=true` で絞れる — e2e: `article_list_filters_by_feed_and_unread`
- 記事一覧は `published_at` の降順、NULL は最後 — テスト無し
- 既読にする操作は何度呼んでも 204 で、行は 1 つ — e2e: `marking_read_is_idempotent_and_read_all_clears_unread`
- 全既読は全フィードの未読を 0 にする — e2e: `marking_read_is_idempotent_and_read_all_clears_unread`
- `GET /api/stats` はフィード数と未読記事数を返す — e2e: `stats_count_feeds_and_unread_articles`
- 記事本文は DOMPurify でサニタイズしてから表示する — テスト無し

### フィードの削除

- フィードを消すとその記事と既読も消え、同じ id の 2 回目は 404 — e2e: `deleting_a_feed_removes_its_articles_and_read_state`
- UI では 3 秒以内に 2 回クリックしたときだけ消す — `ui/src/components/FeedItem.test.tsx`: `deletes only on the second click`, `drops the confirmation after three seconds`

### OPML インポート

- `outline` の `xmlUrl` を入れ子と自己閉じの両方から拾う — `server/src/lib.rs`: `extracts_urls_from_nested_and_self_closing_outlines`
- 空・空白だけ・文字化けした `xmlUrl` は `skipped_invalid` に数え、`xmlUrl` の無い `outline` は数えない。他の属性の文字化けはその URL を落とさない — `server/src/lib.rs`: `ignores_missing_empty_and_whitespace_only_xml_url`, `recovers_urls_when_only_text_attribute_has_invalid_utf8`, `skips_entry_when_xml_url_itself_has_invalid_utf8`
- 壊れた XML はエラー（500） — `server/src/lib.rs`: `errors_on_malformed_xml`（パーサのエラーまで。500 はテスト無し）
- `imported + already_present + skipped_invalid + skipped_blocked` は `xmlUrl` 属性の総数に一致する — テスト無し
- ガードで弾いた URL は `skipped_blocked` に数えて登録しない — テスト無し
- 有効な `xmlUrl` が 1 つも無ければ 400 — テスト無し

### cleaner（#8、#153）

- 消すのは既読かつ `fetched_at` が `retention_days` 日より前の記事だけ。未読と保持期間内の既読は残す。`retention_days` 変数に従う — e2e: `cleaner_deletes_only_read_articles_past_retention`
- `retention_days` の既定は 30 — テスト無し

### DB 接続

- CA があれば `sslmode` を `require` に強制する（他の指定は置き換え、他のオプションは残す）。CA が空なら URL のまま — `shared/src/db.rs`: `appends_when_no_query`, `replaces_a_weaker_mode`, `keeps_unrelated_options`（CA が空のときはテスト無し）
