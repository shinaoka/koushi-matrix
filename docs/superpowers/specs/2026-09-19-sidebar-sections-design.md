# Rooms / DMs セクション型サイドバー詳細設計

Status: implemented baseline — Rooms / DMs の基本 UI、セクション別設定、会話時刻の保護を実装済み。
Date: 2026-09-19
Implementation base: `origin/main`。この文書の現状調査と実装は現在の checkout に基づく。

実装済み範囲は、Rust の `SidebarModel` による Rooms / DMs の対象・順序・開閉状態の投影、
`SettingsPatch.sidebar_section` による Home / Space ごとの設定保存、React の共通見出しと
radio メニュー、会話イベント以外を新着時刻へ昇格させない SDK / Activity 防御である。
アカウント別暗号化ストア、検索結果の Rust 投影、履歴 backfill などは本設計の次段階として残す。

## 1. 目的と決定事項

サイドバーの主領域を、上に Rooms、下に DMs の二つのセクションとして常時配置する。
両方を独立して展開・格納でき、並べ替えは各見出しのメニューで選ぶ。
カテゴリを切り替えて片方だけ表示する現行 UI を置き換える。

- Home と各 Space で、配置・見出し・メニュー・キーボード操作を統一する。
- Rooms と DMs の開閉・並べ替えを独立して保存する。
- 新規設定の既定は両方展開、新着メッセージ順。
- 会話の新着時刻にプロフィール更新などの非会話イベントを混ぜない。
- Rust が対象会話、分類、順序、注意表示、保存設定を所有する。
- React は表示、フォーカス、メニュー、スクロールなどを担当する。

今回追加しないものは、任意セクション作成、セクションのドラッグ移動、手動の会話並べ替え、
端末間での表示設定同期。Home の Activity 画面自体の構成変更も対象外とする。
前の「Rooms / DMs の切り替えボタン順だけを変更する案」は本設計で置き換える。

## 2. 現状と参照契約

調査した現行コード:

- `apps/desktop/src/components/Shell.tsx`: `Sidebar`、`RoomListControls`、`RoomSection`。
- `crates/koushi-state/src/sidebar.rs`: `SidebarModel`、`SidebarSections`、各投影関数。
- `crates/koushi-state/src/state/room.rs`: 会話時刻と attention による比較。
- `crates/koushi-sdk/src/room_projection.rs`: 会話イベントの分類と最新イベント投影。
- `crates/koushi-core/src/runtime/activity.rs`: Activity 画面の最新イベント fallback。

現行 UI は一つの `sidebar.category` と一つの `room_list_sort` を使う。
`Activity` は attention 優先、`RecentFirst` は会話時刻順であり、同じ意味ではない。
現行 UI が両者を `active` にまとめている点は、新メニューでは解消する。
現在の `global_dms` は名前に反して Space 選択時に `dm_space_ids` で絞られる。

参照する規範:

- [Repository Rules](../../../REPOSITORY_RULES.md)
- [Architecture overview](../../architecture/overview.md)
- [State machines](../../architecture/state-machine.md)
- [State ownership](../../agents/state-ownership.md)
- [i18n](../../architecture/i18n.md)

本書は規範文書を上書きしない。実装時には sidebar の分類、設定所有者、検索、遷移に
関する規範を先に整合させ、同じ変更で DTO とテストを更新する。

### 2.1 参考 UI と意図的な差異

ユーザー提示の Slack 画像から採用するのは「縦のセクション」「見出しの操作」「メニュー内の並べ替え」。
画像にある人物名・会話内容は設計資料やテストに転記しない。

ローカル upstream checkout で確認した形:

| 参照 | 確認箇所 | 観察 |
| --- | --- | --- |
| Element Web `aae770e221` | `apps/web/src/components/views/rooms/RoomSublist.tsx` | セクション開閉、見出しメニュー、tag 単位の sorting |
| 同上 | `packages/shared-components/src/room-list/RoomListHeaderView/menu/OptionMenuView.tsx` | recent / unread-first / alphabetical を radio menu として選択 |
| Element X Android `99b7c758f2` | `features/home/impl/.../filters/RoomListFilter.kt`、`RoomListFiltersPresenter.kt` | Rooms / People の排他的フィルターと ToggleFilter intent |
| Element X iOS `8262533` | `ElementX/Sources/Screens/HomeScreen/View/Filters/RoomListFilterModels.swift` | Rooms / People の排他的フィルターから SDK filter へ変換 |

本案はデスクトップで二種類の会話を同時に見せるため、X の排他的カテゴリ操作を採用しない。
upstream の UI コードは移植せず、既存 Koushi の部品と Rust の所有境界に合わせて実装する。

## 3. 情報構造と表示対象

```text
Space 名                          メンバー  設定

会話を絞り込む…

▾ Rooms                           3   ＋  ⋯
    # general
    # development

▾ DMs                             2   ＋  ⋯
    ● Alice
    ● Bob
```

図の会話名・人物名は合成例。見出しの数字は総会話数ではなく、既存 attention 契約に基づく通知数。
Rooms → DMs の順は固定し、DOM 順と視覚順を一致させる。
スクロール領域はサイドバー全体で一つとし、セクションごとの内部スクロールを設けない。

| 表示場所 | Rooms | DMs |
| --- | --- | --- |
| Home | 参加中の非 DM 会話 | アカウント内の参加中 DM |
| Space | 既存 Rust 投影がその Space の対象とする非 DM 会話 | `dm_space_ids` にその Space がある DM |

グループ DM も既存の `is_dm` 判定に従う。React がメンバー数から分類し直さない。
表示名・アイコンが変わっても所属先は変わらない。同じ会話 ID は同一スコープ内で一度だけ表示する。
Home の Activity / Explore / Invites は既存位置に残し、Space に全アカウント用の項目を追加しない。
存在しない Space ID は空の unavailable 状態にし、Home の全会話へ暗黙 fallback しない。

### 3.1 お気に入り・低優先・未参加との整合

二つの主セクションに統一するため、お気に入り・低優先の独立セクションは廃止する。
tag は削除せず、元の Rooms または DMs の行に既存の意味を持つ印として表示する。
お気に入りを先頭へ固定する隠れた順位は入れず、選んだ並べ替えをセクション全体に適用する。
低優先 tag を通知ミュートへ変換しない。tag 変更操作・通知設定は既存 Rust 契約を使う。
この分類変更は本設計で提案する製品判断であり、実装時に規範と tag 移動テストを更新する。

招待は Home の Invites が所有する。未参加ルームの発見・参加は既存 Explore / Space 情報の
導線を維持する。`not_joined` を参加中会話へ混ぜず、実装前に現行の導線を確認する。
現在の sidebar 投影では未参加配列は空だが、それだけを理由に参加機能の入口を削除しない。

## 4. 見出しとメニュー

共通見出しは三つの独立した操作要素からなる。button を入れ子にしない。

| 要素 | Rooms | DMs |
| --- | --- | --- |
| 矢印＋タイトル | 展開／格納 | 展開／格納 |
| ＋ | 現在の Space 文脈でルーム作成 | 現在の文脈で新規 DM |
| ⋯ | Rooms の設定 | DMs の設定 |

＋は既存作成フローを開く。作成・参加・Space 関連付けの権限や結果を UI が推測しない。
ヘッダーにある重複するルーム作成・新規 DM アイコンは各見出しの＋へ集約する。
メンバーや Space 設定など別の対象に属する操作は移動しない。
＋と⋯は常に操作可能な位置に表示し、hover だけに依存しない。

メニューは当初一階層とし、並べ替えの三つの radio 項目を直接置く。

```text
並べ替え
  ✓ 新着メッセージ順
    名前順
    未読・メンション優先
```

チェックは Rust が投影した有効値を表す。選択後に閉じ、対応する⋯へフォーカスを戻す。
同じ値の選択は no-op。変更はそのスコープ・セクションだけに適用する。
メニューの対象は開いた時点の scope と section に固定し、Space／アカウント切り替えで閉じる。
格納状態でもメニューを使用でき、並べ替え変更だけでは展開しない。

## 5. 並べ替えの厳密な定義

| 表示名 | 既存 enum の対応 | 比較順 |
| --- | --- | --- |
| 新着メッセージ順 | `RecentFirst` | 会話時刻あり → 時刻降順 → 名前 → room ID |
| 名前順 | `NormalLocale` | 既存 Rust の表示名比較 → room ID |
| 未読・メンション優先 | `Activity` | 既存 attention rank → 新着メッセージ順 |

名前順は現在の case-folded 表示名順を再利用する。「日本語の読み順」とは表示・説明しない。
未読優先は実効通知設定を反映した既存 rank（メンション、通知、残りの未読、既読）を再利用する。
新着順では未読化・既読化だけで順序を変えない。
名前の変更は名前順や同時刻の tie-break には影響し得るが、新着時刻を作らない。

### 5.1 会話時刻

判定は `conversation_activity` を生成する Rust/SDK 境界に一元化する。
preview 用 `latest_event` と SDK の opaque な `recency_stamp` を新着順の代用にしない。

| 入力 | 新着時刻への扱い |
| --- | --- |
| 通常メッセージ、添付メッセージ、返信 | 対象。メッセージ自身の時刻 |
| スレッド返信 | 対象。返信自身の時刻 |
| 復号前の暗号化メッセージ | 既存分類に従い暫定対象。復号時に再評価 |
| 表示名・アバター変更、参加・退出、room state | 対象外 |
| reaction、edit/replacement、redaction | それ自体の時刻は対象外 |
| receipt、typing、presence | 対象外 |
| 下書き、失敗・取消済みの送信 | 対象外 |

送信中 local echo は現行の会話投影契約に従う暫定候補とし、remote echo と同一送信として
置換する。失敗・取消・復号後の非会話判定では暫定候補を取り除き、既知の有効候補から再計算する。
過去メッセージの遅延到着で最新時刻を後退させない。ただし最新候補の無効化では再計算を許す。
削除された最新メッセージは候補から外し、前の有効メッセージへ戻す。削除操作の時刻は使わない。
履歴が足りなければ unknown を使い、参加時刻や現在時刻で補わない。

不明な履歴を埋めるために全ルームを無制限に backfill しない。既存 cache と購読の範囲で
精度を上げる。SDK に保持される latest がプロフィールイベントでも、以前の有効な会話候補を失わない。
event ID とイベント種別・relation による分類が必要であり、
「latest の時刻 == conversation_activity の時刻」だけでは会話イベントの証明にならない。
異なるイベントが同一ミリ秒に存在する場合も正しく扱う。

前段の未コミット変更にある timestamp 一致による防御は、この契約の完成形とはみなさない。
実装時に権威ある分類へ置き換え、Activity 画面の fallback/read marker にも誤ったイベント ID を渡さない。
Activity 画面の行順変更とサイドバーの room 順変更は別の受け入れ項目として検証する。

## 6. 開閉・絞り込み・選択

開閉は選択中の会話、タイムライン、未読状態を変更しない。
折りたたまれた内容は DOM のフォーカス対象から外す。
内容内にフォーカスがある状態で格納する場合、先に見出しへフォーカスを移す。

会話名の絞り込み欄は一つ。現在のスコープにある両セクションへ同時に適用する。
これはメッセージ本文の全文検索ではない。
入力は共通 IME-safe primitive を使い、合成中 Enter/Escape を会話操作へ転用しない。

- 非空 query では一致があるセクションを一時展開する。
- query 中に利用者が開閉した場合は、その query 世代の表示 override として扱う。
- query が変わると表示 override を破棄し、一致があるセクションを再展開する。
- query 解除で保存済みの開閉へ戻す。query 中の自動展開・手動開閉は保存しない。
- 両見出しは検索中も残す。0 件のセクションには展開時「一致する会話がありません」。
- 見出しの通知数は検索結果件数に変えず、セクション全体の値を維持する。
- Space／アカウント変更で query・メニュー・一時 override をクリアする。

IME draft は React/DOM 所有。検索用の確定 query と一致結果は Rust の一時 UI 投影が所有し、
正規化・名前照合を React と Rust に重複実装しない。query は永続化・ログ出力しない。
非同期結果には scope generation と query revision を付け、古い結果は捨てる。

外部検索、通知、リンクからの明示的な会話移動では、その会話を含む現在スコープなら
query を解除して対象セクションを展開し、行を見える位置へ移動する。この展開は保存する。
対象がスコープ外なら既存の navigation が適切なスコープを決定する。
単なる新着通知・snapshot 更新では格納状態を勝手に変更しない。
現在選択の room ID は並べ替え後も維持し、受信だけで選択行へスクロールしない。

## 7. 空・読込・エラー・注意表示

| 状態 | 表示・操作 |
| --- | --- |
| authoritative な 0 件 | 見出しと＋を表示。展開中は空状態の説明 |
| 初回読込、候補なし | 読込表示。0 件と断定しない |
| 部分キャッシュあり | 既知の行と読み込み状態を表示 |
| 同期失敗 | 既知の行を保持し、既存の失敗／再試行導線を使用 |
| 格納中に未読発生 | 見出しの注意表示だけ更新 |
| ミュート／mentions-only | 既存 `room_attention_projection` の意味に従う |

通知数と highlight の集計は Rust で対象 room ID を重複排除した後に計算する。
折りたたみと絞り込みはその値を変えない。空状態の文言も catalog から取得する。

## 8. 保存モデルと所有者

Space ID を含むため、device-global `SettingsValues` の平文設定へ map を追加しない。
アカウントごとの暗号化ローカル保存領域を使用する。StoreActor が読み書きと session fence を所有する。
表示設定は端末内限定。Matrix account data の room tag と混同しない。

提案する論理モデル（以下の型名は新設案）:

```text
SidebarScope = Home | Space { space_id }
SidebarSectionKind = Rooms | Dms
SectionPreference = { collapsed: bool, sort: RoomListSort }
ScopePreference = { scope, rooms: SectionPreference, dms: SectionPreference }
AccountSidebarPreferences = {
  schema_version,
  imported_legacy,
  default_rooms: SectionPreference,
  default_dms: SectionPreference,
  scopes: [ScopePreference]
}
```

scope は明示的な tagged union とし、空文字などを Home の予約 ID にしない。
アカウントは保存コンテキストで分離し、画面から任意の別アカウントへ書かせない。
未作成 scope は account defaults を使用し、明示的な編集時にエントリーを作る。
一時的に Space が一覧から消えても設定を削除しない。永続データ削除は既存アカウント削除方針に従う。

### 8.1 更新コマンドと競合

`SetSidebarSectionPreference { scope, section, patch, request_id, session_generation }`
を公開 intent とし、patch は `collapsed?` / `sort?` のみを許す。
全 settings オブジェクトを UI が read-modify-write しない。
Ready session、現在有効な scope、許可された enum を検証してから適用する。
同じ値は no-op。保存中でも別フィールドへの操作を受け付け、Rust が最新状態へ合成する。

| 入力 | Rust の結果 |
| --- | --- |
| 有効な変更 | in-memory 設定更新、revision 増加、sidebar 再投影、保存要求 |
| 同一値・重複 request | 意味上の変更なし。既知の結果を維持 |
| 古い session / 無効な scope | 拒否。別スコープへ適用しない |
| 保存成功 | 対応 revision まで persisted と記録 |
| 古い保存完了 | 新しい dirty revision を消さない |
| 保存失敗 | in-memory 値を保持し failed 表示。再試行可能 |
| account switch / logout | 旧 session の結果を新 session に適用しない |

StoreActor はアカウント単位で書き込みを直列化し、待機中の変更を最新 revision へ合成する。
失敗時には「この端末に設定を保存できませんでした」と再試行を既存エラー表示体系で示す。
無限の自動 retry や React timer を追加しない。再試行は最新の未保存状態を対象とする。
保存障害中も会話の閲覧・切り替えは可能。再起動後は最後に保存できた状態になる。

### 8.2 DTO

sidebar には新しい `conversation_sections` を Rooms → DMs 順で投影する。
各要素は `kind`、`preference`、`effective_expanded`、`items`、`total_count`、
`match_count`、`unread_count`、`highlight_count`、`readiness`、作成操作の availability を持つ。
snapshot に scope と query revision、設定保存状態を含める。
`items` は Rust で絞り込み・整列済み。React は配列順に描画する。
旧 `sections` / `space_rooms` / `global_dms` は全 consumer の移行を確認して除去し、二つの投影所有者を残さない。

## 9. 既存設定からの移行

新規インストールは両セクション展開、`RecentFirst`。
既存利用者は明示的な並べ替え意図を保存するため、旧 `room_list_sort` を両セクションの
account default へ一度だけ移す。旧 `Activity` は新メニューでも未読優先のまま対応させる。
既存設定から意図的な選択か旧既定値かを判別できない場合も、勝手に新着順へ変えない。

`sidebar.category` は両方を同時表示するため廃止し、旧選択から片方を格納しない。
旧 favourites / low_priority の格納状態は、会話本体を隠さないため新 Rooms/DMs の格納に転用しない。
tag 自体は保持する。

移行済み marker と新設定は同じ暗号化保存で原子的に確定する。
失敗時は移行完了とせず、旧値の互換読取で表示し、次回再試行する。
完了後は旧設定を再 import して新しい編集を上書きしない。
別アカウントにも一度だけ初期値を適用できるよう、marker はアカウント単位にする。
移行元の旧値は全 consumer の移行期間中だけ互換読取として残し、通常の更新先にしない。
WebView localStorage へ新しい保存処理を追加しない。

## 10. フロントエンド構成・アクセシビリティ

共通 `ConversationSection` と `ConversationSectionHeader`、
`ConversationSectionMenu` を作り、Rooms/DMs の違いは DTO と action target で表す。
既存 `RoomButton`、IME-safe input、浮動レイヤー、menu、tooltip を再利用する。
DOM 上は見出し内の disclosure button、＋ button、menu button を兄弟要素にする。

- disclosure は `aria-expanded` と `aria-controls` を持つ。
- Enter/Space で開閉。＋と⋯には対象を含む catalog-backed accessible name。
- menu は `menuitemradio` / `aria-checked`。矢印キーで移動、Enter で選択、Escape で閉じる。
- メニューは共通 floating layer に置き、sidebar の overflow で切れないようにする。
- hover、focus-visible、選択、未読を既存 design token で表現する。
- 見出し・行の余白は logical CSS properties。長い日本語名でも＋と⋯を押し出さない。
- 初期段階では展開アニメーションを必須にせず、reduced-motion を尊重する。
- 通常の新着並べ替えでは room ID によるスクロール anchor を保つ。

## 11. 実装順序と変更範囲

1. `origin/main` から隔離した実装ブランチを用意する。既存 checkout の未コミット変更を
   まとめて移植せず、本設計に適合する変更だけを比較・移植する。vendor / lockfile の既存差分を保護する。
2. overview / state-machine / state-ownership の sidebar 契約を本設計に合わせる。
3. Phase A: account-scoped 設定、保存・移行、typed command、section projection、
   会話イベント分類と検索を Rust に実装し、headless テストを通す。
4. Phase B: snapshot / delta / IPC / TypeScript mirror / harness を揃え、共通セクション UI を接続する。
5. 不要になったカテゴリ UI、共通 sort UI、旧 section consumer を削除する。
6. ユーザーガイドと `docs/help/settings.md`、必要な生成ドキュメントを更新する。

主な変更領域は `koushi-state` の sidebar/settings/actions/reducers、`koushi-core` の
保存・navigation・projection、`koushi-protocol`、Tauri adapter/DTO、
`Shell.tsx` と関連 UI・catalog・CSS・テスト。
SDK fork の変更は前提にしない。SDK API の不足が判明した場合のみ別途根拠を記録する。

## 12. 受け入れ条件

| 領域 | 必須シナリオ |
| --- | --- |
| 構成 | Home/Space で Rooms が上、DMs が下。両方同時表示 |
| 所属 | Space のルーム・関連 DM だけを表示。お気に入り・低優先も欠落・重複しない |
| 独立設定 | Rooms を名前順、DMs を新着順にしても互いに変更されない |
| 保存 | Space A/B、Home、account A/B、再起動で設定を正しく分離・復元 |
| 保存障害 | 失敗、再試行、連打、古い完了、switch 中の完了で値を上書きしない |
| 新着順 | 新メッセージで対象行が移動。プロフィール変更・reaction・edit・receipt では新着化しない |
| 同時刻 | 同一 ms の state event と message を時刻一致だけで同一視しない |
| 会話候補 | 最新候補の redaction、local echo 失敗、復号後の分類修正、履歴不足を扱う |
| 開閉 | 格納中も注意表示が更新。新着だけで展開・会話移動しない |
| 検索 | 両セクション検索、0 件、query 解除、古い結果、IME、scope 変更 |
| navigation | 外部から対象へ移動した時だけ query 解除・必要な展開・行の reveal |
| UI | ＋/⋯で開閉しない。狭い幅・CJK・RTL・keyboard・focus return を確認 |
| 移行 | 全旧 sort 値、旧カテゴリ、保存失敗、二重起動／再読込で冪等に移行 |
| 境界 | full snapshot と delta で同じ section 値・順序。Tauri command 登録と wire 契約 |

Rust の reducer/projection/store テストで製品の意味を証明し、ブラウザテストは
Rust-shaped snapshot と typed intent のやり取りを検証する。フロント側 fake に並べ替えや移行を実装しない。
新着・profile・relation の実イベント経路は disposable local homeserver の headless QA で確認する。
GUI 目視はレイアウトの補助確認とし、正しさの唯一の根拠にしない。

## 13. 本書作成時の確認記録

現行 sidebar / sort / 設定構造と関連規範、上記 upstream の該当ソースを確認した。
この変更で追加するのは本設計書と plan index へのリンクのみ。アプリコードの変更やテスト実行は含まない。
前段の Rust テストは vendor と依存 API の不一致で停止しているため、実装の検証成功として再利用しない。
