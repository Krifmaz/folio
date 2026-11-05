# Parity

Folio is a port: it must do everything Audiobookshelf (server, web app, mobile app) and Readarr do, the way they
do it, before anything new is added. This file lists every API endpoint, socket event, setting, page, action and
background job of the pinned upstreams (see [UPSTREAM.lock](../UPSTREAM.lock)), one row each.

## Rules

- **Status** is `todo`, `wip`, `done` or `n/a`.
- A row is `done` only with **proof** in its last column: a test (`server/tests/parity_abs_library.rs::items_sort_by_title`)
  or a screenshot (`docs/parity/screens/W0012.png`) showing it works like the original. A test proves an endpoint
  by comparing Folio's answer with the original's, recorded from the pinned upstream (fixtures under
  `server/tests/parity/fixtures/`), status, fields and edge cases included.
- `n/a` needs a reason that comes from [GOALS.md](GOALS.md), never convenience. Today the only `n/a` rows are
  Readarr's indexers and download clients: Folio downloads through Lantern only (GOALS item 2).
- Each row belongs to one **track** (see [TRACKS.md](TRACKS.md)). A track edits only its own rows.
- Settings keep their upstream names, defaults and meaning; pages keep their layout, mobile behaviour and actions.
- After an upstream bump, run `scripts/parity-inventory.py` on the new checkouts, add the new rows, and set rows
  whose upstream source changed back to `todo`.

## Summary (inventory of 2026-11-05)

| Origin | endpoint | socket | setting | page | action | job | provider | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ABS | 222 | 54 | 89 |  |  | 33 |  | 398 |
| ABS web |  |  |  | 42 | 29 |  |  | 71 |
| ABS app |  |  | 24 | 27 | 7 |  |  | 58 |
| Readarr | 234 |  | 92 | 34 | 39 | 10 | 53 | 462 |
| **All** | | | | | | | | **989** |

ID prefixes: `A` Audiobookshelf server (v2.37.2), `W` its web app (`abs-web/`), `M` the mobile app (v0.14.2-beta,
`abs-app/`), `R` Readarr (final `develop`, 2025-06-27). Upstream source paths are relative to each upstream's
repository at the pinned commit.

## Rows

### ABS

| ID | Kind | Item | Upstream source | Track | Status | Proof |
| --- | --- | --- | --- | --- | --- | --- |
| A0001 | endpoint | `POST /api/libraries` (LibraryController.create) | `server/routers/ApiRouter.js#L69` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0002 | endpoint | `GET /api/libraries` (LibraryController.findAll) | `server/routers/ApiRouter.js#L70` | P1 library | wip | `server/tests/parity_abs_library.rs::libraries_answer_like_the_original` (book libraries; podcast stats with P5) |
| A0003 | endpoint | `GET /api/libraries/:id` (LibraryController.findOne) | `server/routers/ApiRouter.js#L71` | P1 library | wip | `server/tests/parity_abs_library.rs::libraries_answer_like_the_original` (book libraries; podcast filter data with P5) |
| A0004 | endpoint | `PATCH /api/libraries/:id` (LibraryController.update) | `server/routers/ApiRouter.js#L72` | P1 library | todo |  |
| A0005 | endpoint | `DELETE /api/libraries/:id` (LibraryController.delete) | `server/routers/ApiRouter.js#L73` | P1 library | todo |  |
| A0006 | endpoint | `GET /api/libraries/:id/items` (LibraryController.getLibraryItems) | `server/routers/ApiRouter.js#L75` | P1 library | wip | `server/tests/parity_abs_library.rs::library_pages_filter_and_sort_like_the_original` (books; podcasts with P5, `rssfeed`/`share` includes with P5) |
| A0007 | endpoint | `DELETE /api/libraries/:id/issues` (LibraryController.removeLibraryItemsWithIssues) | `server/routers/ApiRouter.js#L76` | P1 library | todo |  |
| A0008 | endpoint | `GET /api/libraries/:id/episode-downloads` (LibraryController.getEpisodeDownloadQueue) | `server/routers/ApiRouter.js#L77` | P1 library | todo |  |
| A0009 | endpoint | `GET /api/libraries/:id/series` (LibraryController.getAllSeriesForLibrary) | `server/routers/ApiRouter.js#L78` | P1 library | todo |  |
| A0010 | endpoint | `GET /api/libraries/:id/series/:seriesId` (LibraryController.getSeriesForLibrary) | `server/routers/ApiRouter.js#L79` | P1 library | todo |  |
| A0011 | endpoint | `GET /api/libraries/:id/collections` (LibraryController.getCollectionsForLibrary) | `server/routers/ApiRouter.js#L80` | P1 library | todo |  |
| A0012 | endpoint | `GET /api/libraries/:id/playlists` (LibraryController.getUserPlaylistsForLibrary) | `server/routers/ApiRouter.js#L81` | P1 library | todo |  |
| A0013 | endpoint | `GET /api/libraries/:id/personalized` (LibraryController.getUserPersonalizedShelves) | `server/routers/ApiRouter.js#L82` | P1 library | done | `server/tests/parity_abs_library.rs::home_shelves_answer_like_the_original` |
| A0014 | endpoint | `GET /api/libraries/:id/filterdata` (LibraryController.getLibraryFilterData) | `server/routers/ApiRouter.js#L83` | P1 library | todo |  |
| A0015 | endpoint | `GET /api/libraries/:id/search` (LibraryController.search) | `server/routers/ApiRouter.js#L84` | P1 library | todo |  |
| A0016 | endpoint | `GET /api/libraries/:id/stats` (LibraryController.stats) | `server/routers/ApiRouter.js#L85` | P1 library | todo |  |
| A0017 | endpoint | `GET /api/libraries/:id/authors` (LibraryController.getAuthors) | `server/routers/ApiRouter.js#L86` | P1 library | todo |  |
| A0018 | endpoint | `GET /api/libraries/:id/narrators` (LibraryController.getNarrators) | `server/routers/ApiRouter.js#L87` | P1 library | todo |  |
| A0019 | endpoint | `PATCH /api/libraries/:id/narrators/:narratorId` (LibraryController.updateNarrator) | `server/routers/ApiRouter.js#L88` | P1 library | todo |  |
| A0020 | endpoint | `DELETE /api/libraries/:id/narrators/:narratorId` (LibraryController.removeNarrator) | `server/routers/ApiRouter.js#L89` | P1 library | todo |  |
| A0021 | endpoint | `GET /api/libraries/:id/matchall` (LibraryController.matchAll) | `server/routers/ApiRouter.js#L90` | P1 library | todo |  |
| A0022 | endpoint | `POST /api/libraries/:id/scan` (LibraryController.scan) | `server/routers/ApiRouter.js#L91` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0023 | endpoint | `GET /api/libraries/:id/recent-episodes` (LibraryController.getRecentEpisodes) | `server/routers/ApiRouter.js#L92` | P1 library | todo |  |
| A0024 | endpoint | `GET /api/libraries/:id/opml` (LibraryController.getOPMLFile) | `server/routers/ApiRouter.js#L93` | P1 library | todo |  |
| A0025 | endpoint | `POST /api/libraries/order` (LibraryController.reorder) | `server/routers/ApiRouter.js#L94` | P1 library | todo |  |
| A0026 | endpoint | `POST /api/libraries/:id/remove-metadata` (LibraryController.removeAllMetadataFiles) | `server/routers/ApiRouter.js#L95` | P1 library | todo |  |
| A0027 | endpoint | `GET /api/libraries/:id/podcast-titles` (LibraryController.getPodcastTitles) | `server/routers/ApiRouter.js#L96` | P1 library | todo |  |
| A0028 | endpoint | `GET /api/libraries/:id/download` (LibraryController.downloadMultiple) | `server/routers/ApiRouter.js#L97` | P1 library | todo |  |
| A0029 | endpoint | `POST /api/items/batch/delete` (LibraryItemController.batchDelete) | `server/routers/ApiRouter.js#L102` | P1 library | todo |  |
| A0030 | endpoint | `POST /api/items/batch/update` (LibraryItemController.batchUpdate) | `server/routers/ApiRouter.js#L103` | P1 library | todo |  |
| A0031 | endpoint | `POST /api/items/batch/get` (LibraryItemController.batchGet) | `server/routers/ApiRouter.js#L104` | P1 library | todo |  |
| A0032 | endpoint | `POST /api/items/batch/quickmatch` (LibraryItemController.batchQuickMatch) | `server/routers/ApiRouter.js#L105` | P1 library | todo |  |
| A0033 | endpoint | `POST /api/items/batch/scan` (LibraryItemController.batchScan) | `server/routers/ApiRouter.js#L106` | P1 library | todo |  |
| A0034 | endpoint | `GET /api/items/:id` (LibraryItemController.findOne) | `server/routers/ApiRouter.js#L108` | P1 library | wip | `server/tests/parity_abs_library.rs::items_answer_like_the_original` (books; podcasts with P5, `rssfeed`/`share` includes with P5) |
| A0035 | endpoint | `DELETE /api/items/:id` (LibraryItemController.delete) | `server/routers/ApiRouter.js#L109` | P1 library | todo |  |
| A0036 | endpoint | `GET /api/items/:id/download` (LibraryItemController.download) | `server/routers/ApiRouter.js#L110` | P1 library | todo |  |
| A0037 | endpoint | `PATCH /api/items/:id/media` (LibraryItemController.updateMedia) | `server/routers/ApiRouter.js#L111` | P1 library | todo |  |
| A0038 | endpoint | `GET /api/items/:id/cover` (LibraryItemController.getCover) | `server/routers/ApiRouter.js#L112` | P1 library | todo |  |
| A0039 | endpoint | `POST /api/items/:id/cover` (LibraryItemController.uploadCover) | `server/routers/ApiRouter.js#L113` | P1 library | todo |  |
| A0040 | endpoint | `PATCH /api/items/:id/cover` (LibraryItemController.updateCover) | `server/routers/ApiRouter.js#L114` | P1 library | todo |  |
| A0041 | endpoint | `DELETE /api/items/:id/cover` (LibraryItemController.removeCover) | `server/routers/ApiRouter.js#L115` | P1 library | todo |  |
| A0042 | endpoint | `POST /api/items/:id/match` (LibraryItemController.match) | `server/routers/ApiRouter.js#L116` | P1 library | todo |  |
| A0043 | endpoint | `POST /api/items/:id/play` (LibraryItemController.startPlaybackSession) | `server/routers/ApiRouter.js#L117` | P3 playback | todo |  |
| A0044 | endpoint | `POST /api/items/:id/play/:episodeId` (LibraryItemController.startEpisodePlaybackSession) | `server/routers/ApiRouter.js#L118` | P3 playback | todo |  |
| A0045 | endpoint | `PATCH /api/items/:id/tracks` (LibraryItemController.updateTracks) | `server/routers/ApiRouter.js#L119` | P1 library | todo |  |
| A0046 | endpoint | `POST /api/items/:id/scan` (LibraryItemController.scan) | `server/routers/ApiRouter.js#L120` | P1 library | todo |  |
| A0047 | endpoint | `GET /api/items/:id/metadata-object` (LibraryItemController.getMetadataObject) | `server/routers/ApiRouter.js#L121` | P1 library | todo |  |
| A0048 | endpoint | `POST /api/items/:id/chapters` (LibraryItemController.updateMediaChapters) | `server/routers/ApiRouter.js#L122` | P1 library | todo |  |
| A0049 | endpoint | `GET /api/items/:id/ffprobe/:fileid` (LibraryItemController.getFFprobeData) | `server/routers/ApiRouter.js#L123` | P1 library | todo |  |
| A0050 | endpoint | `GET /api/items/:id/file/:fileid` (LibraryItemController.getLibraryFile) | `server/routers/ApiRouter.js#L124` | P1 library | todo |  |
| A0051 | endpoint | `DELETE /api/items/:id/file/:fileid` (LibraryItemController.deleteLibraryFile) | `server/routers/ApiRouter.js#L125` | P1 library | todo |  |
| A0052 | endpoint | `GET /api/items/:id/file/:fileid/download` (LibraryItemController.downloadLibraryFile) | `server/routers/ApiRouter.js#L126` | P1 library | todo |  |
| A0053 | endpoint | `GET /api/items/:id/ebook/:fileid?` (LibraryItemController.getEBookFile) | `server/routers/ApiRouter.js#L127` | P1 library | todo |  |
| A0054 | endpoint | `PATCH /api/items/:id/ebook/:fileid/status` (LibraryItemController.updateEbookFileStatus) | `server/routers/ApiRouter.js#L128` | P1 library | todo |  |
| A0055 | endpoint | `POST /api/users` (UserController.create) | `server/routers/ApiRouter.js#L133` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0056 | endpoint | `GET /api/users` (UserController.findAll) | `server/routers/ApiRouter.js#L134` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0057 | endpoint | `GET /api/users/online` (UserController.getOnlineUsers) | `server/routers/ApiRouter.js#L135` | P2 users | wip | who is online works; open playback sessions come with P3 playback sessions |
| A0058 | endpoint | `GET /api/users/:id` (UserController.findOne) | `server/routers/ApiRouter.js#L136` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0059 | endpoint | `PATCH /api/users/:id` (UserController.update) | `server/routers/ApiRouter.js#L137` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0060 | endpoint | `DELETE /api/users/:id` (UserController.delete) | `server/routers/ApiRouter.js#L138` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0061 | endpoint | `PATCH /api/users/:id/openid-unlink` (UserController.unlinkFromOpenID) | `server/routers/ApiRouter.js#L139` | P2 users | done | `server/tests/abs_users.rs::users_are_added_read_changed_and_removed_like_the_original` |
| A0062 | endpoint | `GET /api/users/:id/listening-sessions` (UserController.getListeningSessions) | `server/routers/ApiRouter.js#L140` | P3 playback | todo |  |
| A0063 | endpoint | `GET /api/users/:id/listening-stats` (UserController.getListeningStats) | `server/routers/ApiRouter.js#L141` | P3 playback | todo |  |
| A0064 | endpoint | `POST /api/collections` (CollectionController.create) | `server/routers/ApiRouter.js#L146` | P4 collections | todo |  |
| A0065 | endpoint | `GET /api/collections` (CollectionController.findAll) | `server/routers/ApiRouter.js#L147` | P4 collections | todo |  |
| A0066 | endpoint | `GET /api/collections/:id` (CollectionController.findOne) | `server/routers/ApiRouter.js#L148` | P4 collections | todo |  |
| A0067 | endpoint | `PATCH /api/collections/:id` (CollectionController.update) | `server/routers/ApiRouter.js#L149` | P4 collections | todo |  |
| A0068 | endpoint | `DELETE /api/collections/:id` (CollectionController.delete) | `server/routers/ApiRouter.js#L150` | P4 collections | todo |  |
| A0069 | endpoint | `POST /api/collections/:id/book` (CollectionController.addBook) | `server/routers/ApiRouter.js#L151` | P4 collections | todo |  |
| A0070 | endpoint | `DELETE /api/collections/:id/book/:bookId` (CollectionController.removeBook) | `server/routers/ApiRouter.js#L152` | P4 collections | todo |  |
| A0071 | endpoint | `POST /api/collections/:id/batch/add` (CollectionController.addBatch) | `server/routers/ApiRouter.js#L153` | P4 collections | todo |  |
| A0072 | endpoint | `POST /api/collections/:id/batch/remove` (CollectionController.removeBatch) | `server/routers/ApiRouter.js#L154` | P4 collections | todo |  |
| A0073 | endpoint | `POST /api/playlists` (PlaylistController.create) | `server/routers/ApiRouter.js#L159` | P4 collections | todo |  |
| A0074 | endpoint | `GET /api/playlists` (PlaylistController.findAllForUser) | `server/routers/ApiRouter.js#L160` | P4 collections | todo |  |
| A0075 | endpoint | `GET /api/playlists/:id` (PlaylistController.findOne) | `server/routers/ApiRouter.js#L161` | P4 collections | todo |  |
| A0076 | endpoint | `PATCH /api/playlists/:id` (PlaylistController.update) | `server/routers/ApiRouter.js#L162` | P4 collections | todo |  |
| A0077 | endpoint | `DELETE /api/playlists/:id` (PlaylistController.delete) | `server/routers/ApiRouter.js#L163` | P4 collections | todo |  |
| A0078 | endpoint | `POST /api/playlists/:id/item` (PlaylistController.addItem) | `server/routers/ApiRouter.js#L164` | P4 collections | todo |  |
| A0079 | endpoint | `DELETE /api/playlists/:id/item/:libraryItemId/:episodeId?` (PlaylistController.removeItem) | `server/routers/ApiRouter.js#L165` | P4 collections | todo |  |
| A0080 | endpoint | `POST /api/playlists/:id/batch/add` (PlaylistController.addBatch) | `server/routers/ApiRouter.js#L166` | P4 collections | todo |  |
| A0081 | endpoint | `POST /api/playlists/:id/batch/remove` (PlaylistController.removeBatch) | `server/routers/ApiRouter.js#L167` | P4 collections | todo |  |
| A0082 | endpoint | `POST /api/playlists/collection/:collectionId` (PlaylistController.createFromCollection) | `server/routers/ApiRouter.js#L168` | P4 collections | todo |  |
| A0083 | endpoint | `GET /api/me` (MeController.getCurrentUser) | `server/routers/ApiRouter.js#L173` | P2 users | done | `server/tests/abs_auth.rs::sign_in_refresh_and_sign_out_answer_like_the_original` |
| A0084 | endpoint | `GET /api/me/sessions` (MeController.getSessions) | `server/routers/ApiRouter.js#L174` | P2 users | todo |  |
| A0085 | endpoint | `DELETE /api/me/sessions/:id` (MeController.deleteSession) | `server/routers/ApiRouter.js#L175` | P2 users | todo |  |
| A0086 | endpoint | `GET /api/me/progress` (MeController.getAllMediaProgress) | `server/routers/ApiRouter.js#L176` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_follows_the_originals_rules` |
| A0087 | endpoint | `GET /api/me/bookmarks` (MeController.getAllBookmarks) | `server/routers/ApiRouter.js#L177` | P3 playback | done | `server/tests/parity_abs_playback.rs::bookmarks_follow_the_originals_rules` |
| A0088 | endpoint | `GET /api/me/bookmarks/:libraryItemId` (MeController.getBookmarksForLibraryItem) | `server/routers/ApiRouter.js#L178` | P3 playback | done | `server/tests/parity_abs_playback.rs::bookmarks_follow_the_originals_rules` |
| A0089 | endpoint | `GET /api/me/listening-sessions` (MeController.getListeningSessions) | `server/routers/ApiRouter.js#L179` | P3 playback | todo |  |
| A0090 | endpoint | `GET /api/me/item/listening-sessions/:libraryItemId/:episodeId?` (MeController.getItemListeningSessions) | `server/routers/ApiRouter.js#L180` | P3 playback | todo |  |
| A0091 | endpoint | `GET /api/me/listening-stats` (MeController.getListeningStats) | `server/routers/ApiRouter.js#L181` | P3 playback | todo |  |
| A0092 | endpoint | `GET /api/me/progress/:id/remove-from-continue-listening` (MeController.removeItemFromContinueListening) | `server/routers/ApiRouter.js#L182` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_follows_the_originals_rules` |
| A0093 | endpoint | `GET /api/me/progress/:id/:episodeId?` (MeController.getMediaProgress) | `server/routers/ApiRouter.js#L183` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_and_bookmarks_answer_like_the_original` |
| A0094 | endpoint | `PATCH /api/me/progress/batch/update` (MeController.batchUpdateMediaProgress) | `server/routers/ApiRouter.js#L184` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_follows_the_originals_rules` |
| A0095 | endpoint | `PATCH /api/me/progress/:libraryItemId/:episodeId?` (MeController.createUpdateMediaProgress) | `server/routers/ApiRouter.js#L185` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_and_bookmarks_answer_like_the_original` |
| A0096 | endpoint | `DELETE /api/me/progress/:id` (MeController.removeMediaProgress) | `server/routers/ApiRouter.js#L186` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_follows_the_originals_rules` |
| A0097 | endpoint | `POST /api/me/item/:id/bookmark` (MeController.createBookmark) | `server/routers/ApiRouter.js#L187` | P3 playback | done | `server/tests/parity_abs_playback.rs::progress_and_bookmarks_answer_like_the_original` |
| A0098 | endpoint | `PATCH /api/me/item/:id/bookmark` (MeController.updateBookmark) | `server/routers/ApiRouter.js#L188` | P3 playback | done | `server/tests/parity_abs_playback.rs::bookmarks_follow_the_originals_rules` |
| A0099 | endpoint | `DELETE /api/me/item/:id/bookmark/:time` (MeController.removeBookmark) | `server/routers/ApiRouter.js#L189` | P3 playback | done | `server/tests/parity_abs_playback.rs::bookmarks_follow_the_originals_rules` |
| A0100 | endpoint | `PATCH /api/me/password` (MeController.updatePassword) | `server/routers/ApiRouter.js#L190` | P2 users | done | `server/tests/abs_users.rs::a_password_change_signs_out_elsewhere_like_the_original` |
| A0101 | endpoint | `GET /api/me/items-in-progress` (MeController.getAllLibraryItemsInProgress) | `server/routers/ApiRouter.js#L191` | P2 users | todo |  |
| A0102 | endpoint | `GET /api/me/series/:id/remove-from-continue-listening` (MeController.removeSeriesFromContinueListening) | `server/routers/ApiRouter.js#L192` | P2 users | todo |  |
| A0103 | endpoint | `GET /api/me/series/:id/readd-to-continue-listening` (MeController.readdSeriesFromContinueListening) | `server/routers/ApiRouter.js#L193` | P2 users | todo |  |
| A0104 | endpoint | `GET /api/me/stats/year/:year` (MeController.getStatsForYear) | `server/routers/ApiRouter.js#L194` | P2 users | todo |  |
| A0105 | endpoint | `POST /api/me/ereader-devices` (MeController.updateUserEReaderDevices) | `server/routers/ApiRouter.js#L195` | P2 users | todo |  |
| A0106 | endpoint | `GET /api/backups` (BackupController.getAll) | `server/routers/ApiRouter.js#L200` | P6 server admin | todo |  |
| A0107 | endpoint | `POST /api/backups` (BackupController.create) | `server/routers/ApiRouter.js#L201` | P6 server admin | todo |  |
| A0108 | endpoint | `DELETE /api/backups/:id` (BackupController.delete) | `server/routers/ApiRouter.js#L202` | P6 server admin | todo |  |
| A0109 | endpoint | `GET /api/backups/:id/download` (BackupController.download) | `server/routers/ApiRouter.js#L203` | P6 server admin | todo |  |
| A0110 | endpoint | `GET /api/backups/:id/apply` (BackupController.apply) | `server/routers/ApiRouter.js#L204` | P6 server admin | todo |  |
| A0111 | endpoint | `POST /api/backups/upload` (BackupController.upload) | `server/routers/ApiRouter.js#L205` | P6 server admin | todo |  |
| A0112 | endpoint | `PATCH /api/backups/path` (BackupController.updatePath) | `server/routers/ApiRouter.js#L206` | P6 server admin | todo |  |
| A0113 | endpoint | `GET /api/filesystem` (FileSystemController.getPaths) | `server/routers/ApiRouter.js#L211` | P1 library | todo |  |
| A0114 | endpoint | `POST /api/filesystem/pathexists` (FileSystemController.checkPathExists) | `server/routers/ApiRouter.js#L212` | P1 library | todo |  |
| A0115 | endpoint | `GET /api/authors/:id` (AuthorController.findOne) | `server/routers/ApiRouter.js#L217` | P1 library | todo |  |
| A0116 | endpoint | `PATCH /api/authors/:id` (AuthorController.update) | `server/routers/ApiRouter.js#L218` | P1 library | todo |  |
| A0117 | endpoint | `DELETE /api/authors/:id` (AuthorController.delete) | `server/routers/ApiRouter.js#L219` | P1 library | todo |  |
| A0118 | endpoint | `POST /api/authors/:id/match` (AuthorController.match) | `server/routers/ApiRouter.js#L220` | P1 library | todo |  |
| A0119 | endpoint | `GET /api/authors/:id/image` (AuthorController.getImage) | `server/routers/ApiRouter.js#L221` | P1 library | todo |  |
| A0120 | endpoint | `POST /api/authors/:id/image` (AuthorController.uploadImage) | `server/routers/ApiRouter.js#L222` | P1 library | todo |  |
| A0121 | endpoint | `DELETE /api/authors/:id/image` (AuthorController.deleteImage) | `server/routers/ApiRouter.js#L223` | P1 library | todo |  |
| A0122 | endpoint | `GET /api/series/:id` (SeriesController.findOne) | `server/routers/ApiRouter.js#L228` | P1 library | todo |  |
| A0123 | endpoint | `PATCH /api/series/:id` (SeriesController.update) | `server/routers/ApiRouter.js#L229` | P1 library | todo |  |
| A0124 | endpoint | `GET /api/sessions` (SessionController.getAllWithUserData) | `server/routers/ApiRouter.js#L234` | P3 playback | todo |  |
| A0125 | endpoint | `DELETE /api/sessions/:id` (SessionController.delete) | `server/routers/ApiRouter.js#L235` | P3 playback | todo |  |
| A0126 | endpoint | `GET /api/sessions/open` (SessionController.getOpenSessions) | `server/routers/ApiRouter.js#L236` | P3 playback | todo |  |
| A0127 | endpoint | `POST /api/sessions/batch/delete` (SessionController.batchDelete) | `server/routers/ApiRouter.js#L237` | P3 playback | todo |  |
| A0128 | endpoint | `POST /api/session/local` (SessionController.syncLocal) | `server/routers/ApiRouter.js#L238` | P3 playback | todo |  |
| A0129 | endpoint | `POST /api/session/local-all` (SessionController.syncLocalSessions) | `server/routers/ApiRouter.js#L239` | P3 playback | todo |  |
| A0130 | endpoint | `GET /api/session/:id` (SessionController.getOpenSession) | `server/routers/ApiRouter.js#L241` | P3 playback | todo |  |
| A0131 | endpoint | `POST /api/session/:id/sync` (SessionController.sync) | `server/routers/ApiRouter.js#L242` | P3 playback | todo |  |
| A0132 | endpoint | `POST /api/session/:id/close` (SessionController.close) | `server/routers/ApiRouter.js#L243` | P3 playback | todo |  |
| A0133 | endpoint | `POST /api/podcasts` (PodcastController.create) | `server/routers/ApiRouter.js#L248` | P5 podcasts and feeds | todo |  |
| A0134 | endpoint | `POST /api/podcasts/feed` (PodcastController.getPodcastFeed) | `server/routers/ApiRouter.js#L249` | P5 podcasts and feeds | todo |  |
| A0135 | endpoint | `POST /api/podcasts/opml/parse` (PodcastController.getFeedsFromOPMLText) | `server/routers/ApiRouter.js#L250` | P5 podcasts and feeds | todo |  |
| A0136 | endpoint | `POST /api/podcasts/opml/create` (PodcastController.bulkCreatePodcastsFromOpmlFeedUrls) | `server/routers/ApiRouter.js#L251` | P5 podcasts and feeds | todo |  |
| A0137 | endpoint | `GET /api/podcasts/:id/checknew` (PodcastController.checkNewEpisodes) | `server/routers/ApiRouter.js#L252` | P5 podcasts and feeds | todo |  |
| A0138 | endpoint | `GET /api/podcasts/:id/downloads` (PodcastController.getEpisodeDownloads) | `server/routers/ApiRouter.js#L253` | P5 podcasts and feeds | todo |  |
| A0139 | endpoint | `GET /api/podcasts/:id/clear-queue` (PodcastController.clearEpisodeDownloadQueue) | `server/routers/ApiRouter.js#L254` | P5 podcasts and feeds | todo |  |
| A0140 | endpoint | `GET /api/podcasts/:id/search-episode` (PodcastController.findEpisode) | `server/routers/ApiRouter.js#L255` | P5 podcasts and feeds | todo |  |
| A0141 | endpoint | `POST /api/podcasts/:id/download-episodes` (PodcastController.downloadEpisodes) | `server/routers/ApiRouter.js#L256` | P5 podcasts and feeds | todo |  |
| A0142 | endpoint | `POST /api/podcasts/:id/match-episodes` (PodcastController.quickMatchEpisodes) | `server/routers/ApiRouter.js#L257` | P5 podcasts and feeds | todo |  |
| A0143 | endpoint | `GET /api/podcasts/:id/episode/:episodeId` (PodcastController.getEpisode) | `server/routers/ApiRouter.js#L258` | P5 podcasts and feeds | todo |  |
| A0144 | endpoint | `PATCH /api/podcasts/:id/episode/:episodeId` (PodcastController.updateEpisode) | `server/routers/ApiRouter.js#L259` | P5 podcasts and feeds | todo |  |
| A0145 | endpoint | `DELETE /api/podcasts/:id/episode/:episodeId` (PodcastController.removeEpisode) | `server/routers/ApiRouter.js#L260` | P5 podcasts and feeds | todo |  |
| A0146 | endpoint | `GET /api/notifications` (NotificationController.get) | `server/routers/ApiRouter.js#L265` | P6 server admin | todo |  |
| A0147 | endpoint | `PATCH /api/notifications` (NotificationController.update) | `server/routers/ApiRouter.js#L266` | P6 server admin | todo |  |
| A0148 | endpoint | `GET /api/notificationdata` (NotificationController.getData) | `server/routers/ApiRouter.js#L267` | P6 server admin | todo |  |
| A0149 | endpoint | `GET /api/notifications/test` (NotificationController.fireTestEvent) | `server/routers/ApiRouter.js#L268` | P6 server admin | todo |  |
| A0150 | endpoint | `POST /api/notifications` (NotificationController.createNotification) | `server/routers/ApiRouter.js#L269` | P6 server admin | todo |  |
| A0151 | endpoint | `DELETE /api/notifications/:id` (NotificationController.deleteNotification) | `server/routers/ApiRouter.js#L270` | P6 server admin | todo |  |
| A0152 | endpoint | `PATCH /api/notifications/:id` (NotificationController.updateNotification) | `server/routers/ApiRouter.js#L271` | P6 server admin | todo |  |
| A0153 | endpoint | `GET /api/notifications/:id/test` (NotificationController.sendNotificationTest) | `server/routers/ApiRouter.js#L272` | P6 server admin | todo |  |
| A0154 | endpoint | `GET /api/emails/settings` (EmailController.getSettings) | `server/routers/ApiRouter.js#L277` | P6 server admin | todo |  |
| A0155 | endpoint | `PATCH /api/emails/settings` (EmailController.updateSettings) | `server/routers/ApiRouter.js#L278` | P6 server admin | todo |  |
| A0156 | endpoint | `POST /api/emails/test` (EmailController.sendTest) | `server/routers/ApiRouter.js#L279` | P6 server admin | todo |  |
| A0157 | endpoint | `POST /api/emails/ereader-devices` (EmailController.updateEReaderDevices) | `server/routers/ApiRouter.js#L280` | P6 server admin | todo |  |
| A0158 | endpoint | `POST /api/emails/send-ebook-to-device` (EmailController.sendEBookToDevice) | `server/routers/ApiRouter.js#L281` | P6 server admin | todo |  |
| A0159 | endpoint | `GET /api/search/covers` (SearchController.findCovers) | `server/routers/ApiRouter.js#L286` | P1 library | todo |  |
| A0160 | endpoint | `GET /api/search/books` (SearchController.findBooks) | `server/routers/ApiRouter.js#L287` | P1 library | todo |  |
| A0161 | endpoint | `GET /api/search/podcast` (SearchController.findPodcasts) | `server/routers/ApiRouter.js#L288` | P1 library | todo |  |
| A0162 | endpoint | `GET /api/search/authors` (SearchController.findAuthor) | `server/routers/ApiRouter.js#L289` | P1 library | todo |  |
| A0163 | endpoint | `GET /api/search/chapters` (SearchController.findChapters) | `server/routers/ApiRouter.js#L290` | P1 library | todo |  |
| A0164 | endpoint | `GET /api/search/providers` (SearchController.getAllProviders) | `server/routers/ApiRouter.js#L291` | P1 library | todo |  |
| A0165 | endpoint | `POST /api/cache/purge` (CacheController.purgeCache) | `server/routers/ApiRouter.js#L296` | P6 server admin | todo |  |
| A0166 | endpoint | `POST /api/cache/items/purge` (CacheController.purgeItemsCache) | `server/routers/ApiRouter.js#L297` | P6 server admin | todo |  |
| A0167 | endpoint | `POST /api/tools/item/:id/encode-m4b` (ToolsController.encodeM) | `server/routers/ApiRouter.js#L302` | P6 server admin | todo |  |
| A0168 | endpoint | `DELETE /api/tools/item/:id/encode-m4b` (ToolsController.cancelM) | `server/routers/ApiRouter.js#L303` | P6 server admin | todo |  |
| A0169 | endpoint | `POST /api/tools/item/:id/embed-metadata` (ToolsController.embedAudioFileMetadata) | `server/routers/ApiRouter.js#L304` | P6 server admin | todo |  |
| A0170 | endpoint | `POST /api/tools/batch/embed-metadata` (ToolsController.batchEmbedMetadata) | `server/routers/ApiRouter.js#L305` | P6 server admin | todo |  |
| A0171 | endpoint | `GET /api/feeds` (RSSFeedController.getAll) | `server/routers/ApiRouter.js#L310` | P5 podcasts and feeds | todo |  |
| A0172 | endpoint | `POST /api/feeds/item/:itemId/open` (RSSFeedController.openRSSFeedForItem) | `server/routers/ApiRouter.js#L311` | P5 podcasts and feeds | todo |  |
| A0173 | endpoint | `POST /api/feeds/collection/:collectionId/open` (RSSFeedController.openRSSFeedForCollection) | `server/routers/ApiRouter.js#L312` | P5 podcasts and feeds | todo |  |
| A0174 | endpoint | `POST /api/feeds/series/:seriesId/open` (RSSFeedController.openRSSFeedForSeries) | `server/routers/ApiRouter.js#L313` | P5 podcasts and feeds | todo |  |
| A0175 | endpoint | `POST /api/feeds/:id/close` (RSSFeedController.closeRSSFeed) | `server/routers/ApiRouter.js#L314` | P5 podcasts and feeds | todo |  |
| A0176 | endpoint | `GET /api/custom-metadata-providers` (CustomMetadataProviderController.getAll) | `server/routers/ApiRouter.js#L319` | P6 server admin | todo |  |
| A0177 | endpoint | `POST /api/custom-metadata-providers` (CustomMetadataProviderController.create) | `server/routers/ApiRouter.js#L320` | P6 server admin | todo |  |
| A0178 | endpoint | `DELETE /api/custom-metadata-providers/:id` (CustomMetadataProviderController.delete) | `server/routers/ApiRouter.js#L321` | P6 server admin | todo |  |
| A0179 | endpoint | `POST /api/share/mediaitem` (ShareController.createMediaItemShare) | `server/routers/ApiRouter.js#L326` | P6 server admin | todo |  |
| A0180 | endpoint | `DELETE /api/share/mediaitem/:id` (ShareController.deleteMediaItemShare) | `server/routers/ApiRouter.js#L327` | P6 server admin | todo |  |
| A0181 | endpoint | `GET /api/stats/year/:year` (StatsController.getAdminStatsForYear) | `server/routers/ApiRouter.js#L332` | P6 server admin | todo |  |
| A0182 | endpoint | `GET /api/stats/server` (StatsController.getServerStats) | `server/routers/ApiRouter.js#L333` | P6 server admin | todo |  |
| A0183 | endpoint | `GET /api/api-keys` (ApiKeyController.getAll) | `server/routers/ApiRouter.js#L338` | P6 server admin | done | `server/tests/abs_users.rs::api_keys_sign_in_until_turned_off_like_the_original` |
| A0184 | endpoint | `POST /api/api-keys` (ApiKeyController.create) | `server/routers/ApiRouter.js#L339` | P6 server admin | done | `server/tests/abs_users.rs::api_keys_sign_in_until_turned_off_like_the_original` |
| A0185 | endpoint | `PATCH /api/api-keys/:id` (ApiKeyController.update) | `server/routers/ApiRouter.js#L340` | P6 server admin | done | `server/tests/abs_users.rs::api_keys_sign_in_until_turned_off_like_the_original` |
| A0186 | endpoint | `DELETE /api/api-keys/:id` (ApiKeyController.delete) | `server/routers/ApiRouter.js#L341` | P6 server admin | done | `server/tests/abs_users.rs::api_keys_sign_in_until_turned_off_like_the_original` |
| A0187 | endpoint | `POST /api/upload` (MiscController.handleUpload) | `server/routers/ApiRouter.js#L346` | P6 server admin | todo |  |
| A0188 | endpoint | `GET /api/tasks` (MiscController.getTasks) | `server/routers/ApiRouter.js#L347` | P6 server admin | todo |  |
| A0189 | endpoint | `PATCH /api/settings` (MiscController.updateServerSettings) | `server/routers/ApiRouter.js#L348` | P6 server admin | todo |  |
| A0190 | endpoint | `PATCH /api/sorting-prefixes` (MiscController.updateSortingPrefixes) | `server/routers/ApiRouter.js#L349` | P6 server admin | todo |  |
| A0191 | endpoint | `POST /api/authorize` (MiscController.authorize) | `server/routers/ApiRouter.js#L350` | P2 users | done | `server/tests/abs_auth.rs::sign_in_refresh_and_sign_out_answer_like_the_original` |
| A0192 | endpoint | `GET /api/tags` (MiscController.getAllTags) | `server/routers/ApiRouter.js#L351` | P1 library | todo |  |
| A0193 | endpoint | `POST /api/tags/rename` (MiscController.renameTag) | `server/routers/ApiRouter.js#L352` | P1 library | todo |  |
| A0194 | endpoint | `DELETE /api/tags/:tag` (MiscController.deleteTag) | `server/routers/ApiRouter.js#L353` | P1 library | todo |  |
| A0195 | endpoint | `GET /api/genres` (MiscController.getAllGenres) | `server/routers/ApiRouter.js#L354` | P1 library | todo |  |
| A0196 | endpoint | `POST /api/genres/rename` (MiscController.renameGenre) | `server/routers/ApiRouter.js#L355` | P1 library | todo |  |
| A0197 | endpoint | `DELETE /api/genres/:genre` (MiscController.deleteGenre) | `server/routers/ApiRouter.js#L356` | P1 library | todo |  |
| A0198 | endpoint | `POST /api/validate-cron` (MiscController.validateCronExpression) | `server/routers/ApiRouter.js#L357` | P6 server admin | todo |  |
| A0199 | endpoint | `GET /api/auth-settings` (MiscController.getAuthSettings) | `server/routers/ApiRouter.js#L358` | P2 users | done | `server/tests/abs_users.rs::sign_in_settings_are_checked_and_saved_like_the_original` |
| A0200 | endpoint | `PATCH /api/auth-settings` (MiscController.updateAuthSettings) | `server/routers/ApiRouter.js#L359` | P2 users | done | `server/tests/abs_users.rs::sign_in_settings_are_checked_and_saved_like_the_original` |
| A0201 | endpoint | `POST /api/watcher/update` (MiscController.updateWatchedPath) | `server/routers/ApiRouter.js#L360` | P6 server admin | todo |  |
| A0202 | endpoint | `GET /api/logger-data` (MiscController.getLoggerData) | `server/routers/ApiRouter.js#L361` | P6 server admin | todo |  |
| A0203 | endpoint | `GET /hls/:stream/:file` | `server/routers/HlsRouter.js#L21` | P3 playback | todo |  |
| A0204 | endpoint | `GET /public/share/:slug` | `server/routers/PublicRouter.js#L16` | P6 server admin | todo |  |
| A0205 | endpoint | `GET /public/share/:slug/track/:index` | `server/routers/PublicRouter.js#L17` | P6 server admin | todo |  |
| A0206 | endpoint | `GET /public/share/:slug/cover` | `server/routers/PublicRouter.js#L18` | P6 server admin | todo |  |
| A0207 | endpoint | `GET /public/share/:slug/download` | `server/routers/PublicRouter.js#L19` | P6 server admin | todo |  |
| A0208 | endpoint | `PATCH /public/share/:slug/progress` | `server/routers/PublicRouter.js#L20` | P3 playback | todo |  |
| A0209 | endpoint | `GET /public/session/:id/track/:index` | `server/routers/PublicRouter.js#L21` | P3 playback | todo |  |
| A0210 | endpoint | `GET /feed/:slug` | `server/Server.js#L362` | P5 podcasts and feeds | todo |  |
| A0211 | endpoint | `GET /feed/:slug/cover*` | `server/Server.js#L366` | P5 podcasts and feeds | todo |  |
| A0212 | endpoint | `GET /feed/:slug/item/:episodeId/*` | `server/Server.js#L369` | P5 podcasts and feeds | todo |  |
| A0213 | endpoint | `POST /init` | `server/Server.js#L377` | P2 users | done | `server/tests/abs_auth.rs::a_new_server_asks_for_setup_then_answers_like_the_original` |
| A0214 | endpoint | `GET /status` | `server/Server.js#L384` | P2 users | done | `server/tests/abs_auth.rs::a_new_server_asks_for_setup_then_answers_like_the_original` |
| A0215 | endpoint | `GET /ping` | `server/Server.js#L401` | P2 users | done | `server/tests/abs_auth.rs::a_new_server_asks_for_setup_then_answers_like_the_original` |
| A0216 | endpoint | `GET /healthcheck` | `server/Server.js#L405` | P2 users | done | `server/tests/abs_auth.rs::a_new_server_asks_for_setup_then_answers_like_the_original` |
| A0217 | endpoint | `POST /login` | `server/Auth.js#L322` | P2 users | done | `server/tests/abs_auth.rs::sign_in_refresh_and_sign_out_answer_like_the_original`, `server/tests/abs_auth.rs::too_many_sign_in_attempts_are_refused`, `server/tests/abs_auth.rs::a_passwordless_root_signs_in_with_an_empty_password_only` |
| A0218 | endpoint | `POST /auth/refresh` | `server/Auth.js#L331` | P2 users | done | `server/tests/abs_auth.rs::sign_in_refresh_and_sign_out_answer_like_the_original` |
| A0219 | endpoint | `GET /auth/openid` | `server/Auth.js#L362` | P2 users | todo |  |
| A0220 | endpoint | `GET /auth/openid/mobile-redirect` | `server/Auth.js#L380` | P2 users | todo |  |
| A0221 | endpoint | `GET /auth/openid/config` | `server/Auth.js#L464` | P2 users | todo |  |
| A0222 | endpoint | `POST /logout` | `server/Auth.js#L486` | P2 users | wip | `server/tests/abs_auth.rs::sign_in_refresh_and_sign_out_answer_like_the_original`; the OpenID sign-out address comes with P2b |
| A0223 | socket | client → server `auth` | `server/SocketAuthority.js#L188` | P2 users | done | `server/tests/abs_socket.rs::a_valid_token_signs_the_socket_in_like_the_original`, `server/tests/abs_socket.rs::apps_connect_and_a_bad_token_gets_auth_failed` |
| A0224 | socket | client → server `cancel_scan` | `server/SocketAuthority.js#L191` | P2 users | todo |  |
| A0225 | socket | client → server `search_covers` | `server/SocketAuthority.js#L197` | P2 users | todo |  |
| A0226 | socket | client → server `cancel_cover_search` | `server/SocketAuthority.js#L198` | P2 users | todo |  |
| A0227 | socket | client → server `set_log_listener` | `server/SocketAuthority.js#L201` | P2 users | todo |  |
| A0228 | socket | client → server `remove_log_listener` | `server/SocketAuthority.js#L210` | P2 users | todo |  |
| A0229 | socket | client → server `disconnect` | `server/SocketAuthority.js#L213` | P2 users | wip | `user_offline` is sent to admins; no test yet |
| A0230 | socket | client → server `message_all_users` | `server/SocketAuthority.js#L239` | P2 users | todo |  |
| A0231 | socket | client → server `ping` | `server/SocketAuthority.js#L249` | P2 users | done | `server/tests/abs_socket.rs::apps_connect_and_a_bad_token_gets_auth_failed` |
| A0232 | socket | server → client `admin_message` | `server/SocketAuthority.js#L244` | P2 users | todo |  |
| A0233 | socket | server → client `author_added` | `server/models/Book.js#L471` | P2 users | todo |  |
| A0234 | socket | server → client `author_removed` | `server/controllers/AuthorController.js#L162` | P2 users | todo |  |
| A0235 | socket | server → client `author_updated` | `server/controllers/AuthorController.js#L168` | P2 users | todo |  |
| A0236 | socket | server → client `authors_num_books_updated` | `server/scanner/BookScanner.js#L926` | P2 users | todo |  |
| A0237 | socket | server → client `backup_applied` | `server/managers/BackupManager.js#L266` | P2 users | todo |  |
| A0238 | socket | server → client `collection_added` | `server/controllers/CollectionController.js#L104` | P2 users | todo |  |
| A0239 | socket | server → client `collection_removed` | `server/controllers/CollectionController.js#L222` | P2 users | todo |  |
| A0240 | socket | server → client `collection_updated` | `server/controllers/CollectionController.js#L201` | P2 users | todo |  |
| A0241 | socket | server → client `custom_metadata_provider_added` | `server/controllers/CustomMetadataProviderController.js#L59` | P2 users | todo |  |
| A0242 | socket | server → client `custom_metadata_provider_removed` | `server/controllers/CustomMetadataProviderController.js#L96` | P2 users | todo |  |
| A0243 | socket | server → client `episode_added` | `server/managers/PodcastManager.js#L256` | P2 users | todo |  |
| A0244 | socket | server → client `episode_download_finished` | `server/managers/PodcastManager.js#L177` | P2 users | todo |  |
| A0245 | socket | server → client `episode_download_queue_cleared` | `server/managers/PodcastManager.js#L50` | P2 users | todo |  |
| A0246 | socket | server → client `episode_download_queued` | `server/managers/PodcastManager.js#L85` | P2 users | todo |  |
| A0247 | socket | server → client `episode_download_started` | `server/managers/PodcastManager.js#L104` | P2 users | todo |  |
| A0248 | socket | server → client `item_added` | `server/controllers/PodcastController.js#L181` | P2 users | todo |  |
| A0249 | socket | server → client `item_removed` | `server/routers/ApiRouter.js#L403` | P2 users | todo |  |
| A0250 | socket | server → client `item_updated` | `server/controllers/MiscController.js#L377` | P2 users | todo |  |
| A0251 | socket | server → client `items_added` | `server/scanner/LibraryScanner.js#L271` | P2 users | todo |  |
| A0252 | socket | server → client `items_updated` | `server/controllers/LibraryController.js#L1189` | P2 users | todo |  |
| A0253 | socket | server → client `library_added` | `server/controllers/LibraryController.js#L171` | P2 users | todo |  |
| A0254 | socket | server → client `library_removed` | `server/controllers/LibraryController.js#L588` | P2 users | todo |  |
| A0255 | socket | server → client `library_updated` | `server/controllers/LibraryController.js#L511` | P2 users | todo |  |
| A0256 | socket | server → client `metadata_embed_queue_update` | `server/managers/AudioMetadataManager.js#L119` | P2 users | todo |  |
| A0257 | socket | server → client `notifications_updated` | `server/managers/NotificationManager.js#L175` | P2 users | todo |  |
| A0258 | socket | server → client `rss_feed_closed` | `server/managers/RssFeedManager.js#L358` | P2 users | todo |  |
| A0259 | socket | server → client `rss_feed_open` | `server/managers/RssFeedManager.js#L302` | P2 users | todo |  |
| A0260 | socket | server → client `series_added` | `server/scanner/Scanner.js#L316` | P2 users | todo |  |
| A0261 | socket | server → client `series_removed` | `server/routers/ApiRouter.js#L453` | P2 users | todo |  |
| A0262 | socket | server → client `series_updated` | `server/controllers/SeriesController.js#L84` | P2 users | todo |  |
| A0263 | socket | server → client `share_closed` | `server/managers/ShareManager.js#L165` | P2 users | todo |  |
| A0264 | socket | server → client `share_open` | `server/managers/ShareManager.js#L142` | P2 users | todo |  |
| A0265 | socket | server → client `stream_reset` | `server/routers/HlsRouter.js#L87` | P2 users | todo |  |
| A0266 | socket | server → client `task_finished` | `server/managers/TaskManager.js#L35` | P2 users | todo |  |
| A0267 | socket | server → client `task_progress` | `server/managers/AbMergeManager.js#L148` | P2 users | todo |  |
| A0268 | socket | server → client `task_started` | `server/managers/TaskManager.js#L24` | P2 users | todo |  |
| A0269 | socket | server → client `track_finished` | `server/managers/AbMergeManager.js#L150` | P2 users | todo |  |
| A0270 | socket | server → client `track_progress` | `server/managers/AbMergeManager.js#L147` | P2 users | todo |  |
| A0271 | socket | server → client `track_started` | `server/managers/AbMergeManager.js#L145` | P2 users | todo |  |
| A0272 | socket | server → client `user_added` | `server/controllers/UserController.js#L187` | P2 users | todo |  |
| A0273 | socket | server → client `user_offline` | `server/SocketAuthority.js#L224` | P2 users | wip | sent on disconnect; no test yet |
| A0274 | socket | server → client `user_online` | `server/SocketAuthority.js#L348` | P2 users | done | `server/tests/abs_socket.rs::a_valid_token_signs_the_socket_in_like_the_original` |
| A0275 | socket | server → client `user_removed` | `server/controllers/UserController.js#L402` | P2 users | todo |  |
| A0276 | socket | server → client `user_stream_update` | `server/managers/PlaybackSessionManager.js#L361` | P2 users | todo |  |
| A0277 | setting | server `tokenSecret` (default `null`) | `server/objects/settings/ServerSettings.js#L36` | P6 server admin | todo |  |
| A0278 | setting | server `scannerParseSubtitle` (default `false`) | `server/objects/settings/ServerSettings.js#L39` | P6 server admin | todo |  |
| A0279 | setting | server `scannerFindCovers` (default `false`) | `server/objects/settings/ServerSettings.js#L40` | P6 server admin | todo |  |
| A0280 | setting | server `scannerCoverProvider` (default `'google'`) | `server/objects/settings/ServerSettings.js#L41` | P6 server admin | todo |  |
| A0281 | setting | server `scannerPreferMatchedMetadata` (default `false`) | `server/objects/settings/ServerSettings.js#L42` | P6 server admin | todo |  |
| A0282 | setting | server `scannerDisableWatcher` (default `false`) | `server/objects/settings/ServerSettings.js#L43` | P6 server admin | todo |  |
| A0283 | setting | server `storeCoverWithItem` (default `false`) | `server/objects/settings/ServerSettings.js#L46` | P6 server admin | todo |  |
| A0284 | setting | server `storeMetadataWithItem` (default `false`) | `server/objects/settings/ServerSettings.js#L47` | P6 server admin | todo |  |
| A0285 | setting | server `metadataFileFormat` (default `'json'`) | `server/objects/settings/ServerSettings.js#L48` | P6 server admin | todo |  |
| A0286 | setting | server `rateLimitLoginRequests` (default `10`) | `server/objects/settings/ServerSettings.js#L51` | P6 server admin | todo |  |
| A0287 | setting | server `rateLimitLoginWindow` (default `10 * 60 * 1000`) | `server/objects/settings/ServerSettings.js#L52` | P6 server admin | todo |  |
| A0288 | setting | server `allowIframe` (default `false`) | `server/objects/settings/ServerSettings.js#L53` | P6 server admin | todo |  |
| A0289 | setting | server `backupPath` (default `Path.join(global.MetadataPath, 'backups'`) | `server/objects/settings/ServerSettings.js#L56` | P6 server admin | todo |  |
| A0290 | setting | server `backupSchedule` (default `false`) | `server/objects/settings/ServerSettings.js#L57` | P6 server admin | todo |  |
| A0291 | setting | server `backupsToKeep` (default `2`) | `server/objects/settings/ServerSettings.js#L58` | P6 server admin | todo |  |
| A0292 | setting | server `maxBackupSize` (default `1`) | `server/objects/settings/ServerSettings.js#L59` | P6 server admin | todo |  |
| A0293 | setting | server `loggerDailyLogsToKeep` (default `7`) | `server/objects/settings/ServerSettings.js#L62` | P6 server admin | todo |  |
| A0294 | setting | server `loggerScannerLogsToKeep` (default `2`) | `server/objects/settings/ServerSettings.js#L63` | P6 server admin | todo |  |
| A0295 | setting | server `homeBookshelfView` (default `BookshelfView.DETAIL`) | `server/objects/settings/ServerSettings.js#L66` | P6 server admin | todo |  |
| A0296 | setting | server `bookshelfView` (default `BookshelfView.DETAIL`) | `server/objects/settings/ServerSettings.js#L67` | P6 server admin | todo |  |
| A0297 | setting | server `podcastEpisodeSchedule` (default `'0 * * * *'`) | `server/objects/settings/ServerSettings.js#L70` | P6 server admin | todo |  |
| A0298 | setting | server `sortingIgnorePrefix` (default `false`) | `server/objects/settings/ServerSettings.js#L73` | P6 server admin | todo |  |
| A0299 | setting | server `sortingPrefixes` (default `['the', 'a']`) | `server/objects/settings/ServerSettings.js#L74` | P6 server admin | todo |  |
| A0300 | setting | server `chromecastEnabled` (default `false`) | `server/objects/settings/ServerSettings.js#L77` | P6 server admin | todo |  |
| A0301 | setting | server `dateFormat` (default `'MM/dd/yyyy'`) | `server/objects/settings/ServerSettings.js#L78` | P6 server admin | todo |  |
| A0302 | setting | server `timeFormat` (default `'HH:mm'`) | `server/objects/settings/ServerSettings.js#L79` | P6 server admin | todo |  |
| A0303 | setting | server `language` (default `'en-us'`) | `server/objects/settings/ServerSettings.js#L80` | P6 server admin | todo |  |
| A0304 | setting | server `allowedOrigins` (default `[]`) | `server/objects/settings/ServerSettings.js#L81` | P6 server admin | todo |  |
| A0305 | setting | server `logLevel` (default `Logger.logLevel`) | `server/objects/settings/ServerSettings.js#L83` | P6 server admin | todo |  |
| A0306 | setting | server `version` (default `packageJson.version`) | `server/objects/settings/ServerSettings.js#L85` | P6 server admin | todo |  |
| A0307 | setting | server `buildNumber` (default `packageJson.buildNumber`) | `server/objects/settings/ServerSettings.js#L86` | P6 server admin | todo |  |
| A0308 | setting | server `authLoginCustomMessage` (default `null`) | `server/objects/settings/ServerSettings.js#L89` | P6 server admin | todo |  |
| A0309 | setting | server `authActiveAuthMethods` (default `['local']`) | `server/objects/settings/ServerSettings.js#L90` | P6 server admin | todo |  |
| A0310 | setting | server `authOpenIDIssuerURL` (default `null`) | `server/objects/settings/ServerSettings.js#L93` | P6 server admin | todo |  |
| A0311 | setting | server `authOpenIDAuthorizationURL` (default `null`) | `server/objects/settings/ServerSettings.js#L94` | P6 server admin | todo |  |
| A0312 | setting | server `authOpenIDTokenURL` (default `null`) | `server/objects/settings/ServerSettings.js#L95` | P6 server admin | todo |  |
| A0313 | setting | server `authOpenIDUserInfoURL` (default `null`) | `server/objects/settings/ServerSettings.js#L96` | P6 server admin | todo |  |
| A0314 | setting | server `authOpenIDJwksURL` (default `null`) | `server/objects/settings/ServerSettings.js#L97` | P6 server admin | todo |  |
| A0315 | setting | server `authOpenIDLogoutURL` (default `null`) | `server/objects/settings/ServerSettings.js#L98` | P6 server admin | todo |  |
| A0316 | setting | server `authOpenIDClientID` (default `null`) | `server/objects/settings/ServerSettings.js#L99` | P6 server admin | todo |  |
| A0317 | setting | server `authOpenIDClientSecret` (default `null`) | `server/objects/settings/ServerSettings.js#L100` | P6 server admin | todo |  |
| A0318 | setting | server `authOpenIDTokenSigningAlgorithm` (default `'RS256'`) | `server/objects/settings/ServerSettings.js#L101` | P6 server admin | todo |  |
| A0319 | setting | server `authOpenIDButtonText` (default `'Login with OpenId'`) | `server/objects/settings/ServerSettings.js#L102` | P6 server admin | todo |  |
| A0320 | setting | server `authOpenIDAutoLaunch` (default `false`) | `server/objects/settings/ServerSettings.js#L103` | P6 server admin | todo |  |
| A0321 | setting | server `authOpenIDAutoRegister` (default `false`) | `server/objects/settings/ServerSettings.js#L104` | P6 server admin | todo |  |
| A0322 | setting | server `authOpenIDMatchExistingBy` (default `null`) | `server/objects/settings/ServerSettings.js#L105` | P6 server admin | todo |  |
| A0323 | setting | server `authOpenIDMobileRedirectURIs` (default `['audiobookshelf:`) | `server/objects/settings/ServerSettings.js#L106` | P6 server admin | todo |  |
| A0324 | setting | server `authOpenIDGroupClaim` (default `''`) | `server/objects/settings/ServerSettings.js#L107` | P6 server admin | todo |  |
| A0325 | setting | server `authOpenIDAdvancedPermsClaim` (default `''`) | `server/objects/settings/ServerSettings.js#L108` | P6 server admin | todo |  |
| A0326 | setting | server `authOpenIDSubfolderForRedirectURLs` (default `undefined`) | `server/objects/settings/ServerSettings.js#L109` | P6 server admin | todo |  |
| A0327 | setting | email `host` | `server/objects/settings/EmailSettings.js#L16` | P6 server admin | todo |  |
| A0328 | setting | email `port` | `server/objects/settings/EmailSettings.js#L17` | P6 server admin | todo |  |
| A0329 | setting | email `secure` | `server/objects/settings/EmailSettings.js#L18` | P6 server admin | todo |  |
| A0330 | setting | email `rejectUnauthorized` | `server/objects/settings/EmailSettings.js#L19` | P6 server admin | todo |  |
| A0331 | setting | email `user` | `server/objects/settings/EmailSettings.js#L20` | P6 server admin | todo |  |
| A0332 | setting | email `pass` | `server/objects/settings/EmailSettings.js#L21` | P6 server admin | todo |  |
| A0333 | setting | email `testAddress` | `server/objects/settings/EmailSettings.js#L22` | P6 server admin | todo |  |
| A0334 | setting | email `fromAddress` | `server/objects/settings/EmailSettings.js#L23` | P6 server admin | todo |  |
| A0335 | setting | email `ereaderDevices` | `server/objects/settings/EmailSettings.js#L26` | P6 server admin | todo |  |
| A0336 | setting | notifications `appriseType` | `server/objects/settings/NotificationSettings.js#L8` | P6 server admin | todo |  |
| A0337 | setting | notifications `appriseApiUrl` | `server/objects/settings/NotificationSettings.js#L9` | P6 server admin | todo |  |
| A0338 | setting | notifications `notifications` | `server/objects/settings/NotificationSettings.js#L10` | P6 server admin | todo |  |
| A0339 | setting | notifications `maxFailedAttempts` | `server/objects/settings/NotificationSettings.js#L11` | P6 server admin | todo |  |
| A0340 | setting | notifications `maxNotificationQueue` | `server/objects/settings/NotificationSettings.js#L12` | P6 server admin | todo |  |
| A0341 | setting | notifications `notificationDelay` | `server/objects/settings/NotificationSettings.js#L13` | P6 server admin | todo |  |
| A0342 | setting | library `coverAspectRatio` | `server/models/Library.js#L59` | P1 library | todo |  |
| A0343 | setting | library `disableWatcher` | `server/models/Library.js#L60` | P1 library | todo |  |
| A0344 | setting | library `autoScanCronExpression` | `server/models/Library.js#L61` | P1 library | todo |  |
| A0345 | setting | library `podcastSearchRegion` | `server/models/Library.js#L62` | P1 library | todo |  |
| A0346 | setting | library `markAsFinishedPercentComplete` | `server/models/Library.js#L63` | P1 library | todo |  |
| A0347 | setting | library `markAsFinishedTimeRemaining` | `server/models/Library.js#L64` | P1 library | todo |  |
| A0348 | setting | library `skipMatchingMediaWithAsin` | `server/models/Library.js#L71` | P1 library | todo |  |
| A0349 | setting | library `skipMatchingMediaWithIsbn` | `server/models/Library.js#L72` | P1 library | todo |  |
| A0350 | setting | library `audiobooksOnly` | `server/models/Library.js#L73` | P1 library | todo |  |
| A0351 | setting | library `epubsAllowScriptedContent` | `server/models/Library.js#L74` | P1 library | todo |  |
| A0352 | setting | library `hideSingleBookSeries` | `server/models/Library.js#L75` | P1 library | todo |  |
| A0353 | setting | library `onlyShowLaterBooksInContinueSeries` | `server/models/Library.js#L76` | P1 library | todo |  |
| A0354 | setting | library `metadataPrecedence` | `server/models/Library.js#L77` | P1 library | todo |  |
| A0355 | setting | user permission `download` | `server/models/User.js#L171` | P2 users | todo |  |
| A0356 | setting | user permission `update` | `server/models/User.js#L172` | P2 users | todo |  |
| A0357 | setting | user permission `delete` | `server/models/User.js#L173` | P2 users | todo |  |
| A0358 | setting | user permission `upload` | `server/models/User.js#L174` | P2 users | todo |  |
| A0359 | setting | user permission `createEreader` | `server/models/User.js#L175` | P2 users | todo |  |
| A0360 | setting | user permission `accessAllLibraries` | `server/models/User.js#L176` | P2 users | todo |  |
| A0361 | setting | user permission `accessAllTags` | `server/models/User.js#L177` | P2 users | todo |  |
| A0362 | setting | user permission `accessExplicitContent` | `server/models/User.js#L178` | P2 users | todo |  |
| A0363 | setting | user permission `selectedTagsNotAccessible` | `server/models/User.js#L179` | P2 users | todo |  |
| A0364 | setting | user permission `librariesAccessible` | `server/models/User.js#L180` | P2 users | todo |  |
| A0365 | setting | user permission `itemTagsSelected` | `server/models/User.js#L181` | P2 users | todo |  |
| A0366 | job | notification event `onPodcastEpisodeDownloaded` | `server/utils/notifications.js#L6` | P6 server admin | todo |  |
| A0367 | job | notification event `onBackupCompleted` | `server/utils/notifications.js#L32` | P6 server admin | todo |  |
| A0368 | job | notification event `onBackupFailed` | `server/utils/notifications.js#L50` | P6 server admin | todo |  |
| A0369 | job | notification event `onRSSFeedFailed` | `server/utils/notifications.js#L64` | P6 server admin | todo |  |
| A0370 | job | notification event `onRSSFeedDisabled` | `server/utils/notifications.js#L80` | P6 server admin | todo |  |
| A0371 | job | notification event `onTest` | `server/utils/notifications.js#L96` | P6 server admin | todo |  |
| A0372 | job | library scan schedules (`cronExpression` per library) | `server/managers/CronManager.js` | P1 library | todo |  |
| A0373 | job | podcast episode checks (`podcastEpisodeSchedule`, per podcast cron) | `server/managers/CronManager.js` | P5 podcasts and feeds | todo |  |
| A0374 | job | scheduled backups (`backupSchedule`, `backupsToKeep`, `maxBackupSize`) | `server/managers/BackupManager.js` | P6 server admin | todo |  |
| A0375 | job | daily log files and rotation (`loggerDailyLogsToKeep`, `loggerScannerLogsToKeep`) | `server/managers/LogManager.js` | P6 server admin | todo |  |
| A0376 | job | folder watcher: rescans changed items (`scannerDisableWatcher`) | `server/Watcher.js` | P1 library | todo |  |
| A0377 | job | stale open sessions closed (`removeOrphanStreams`, idle sessions) | `server/managers/PlaybackSessionManager.js` | P3 playback | todo |  |
| A0378 | job | embed-metadata queue (Quick Embed) | `server/managers/AudioMetadataManager.js` | P6 server admin | todo |  |
| A0379 | job | M4B merge tasks | `server/managers/AbMergeManager.js` | P6 server admin | todo |  |
| A0380 | job | cover and item cache purge | `server/managers/CacheManager.js` | P6 server admin | todo |  |
| A0381 | job | open RSS feeds kept in step with items | `server/managers/RssFeedManager.js` | P5 podcasts and feeds | todo |  |
| A0382 | job | episode download queue | `server/managers/PodcastManager.js` | P5 podcasts and feeds | todo |  |
| A0383 | job | database migrations on start | `server/managers/MigrationManager.js` | P6 server admin | todo |  |
| A0384 | job | ffmpeg/ffprobe presence check and download | `server/managers/BinaryManager.js` | P6 server admin | todo |  |
| A0385 | job | server start: purge (`cleanUserData`, `purgeMetadata`) | `server/Server.js` | P6 server admin | todo |  |
| A0386 | job | scanner: `AbsMetadataFileScanner` | `server/scanner/AbsMetadataFileScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0387 | job | scanner: `AudioFileScanner` | `server/scanner/AudioFileScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0388 | job | scanner: `BookScanner` | `server/scanner/BookScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0389 | job | scanner: `LibraryItemScanData` | `server/scanner/LibraryItemScanData.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0390 | job | scanner: `LibraryItemScanner` | `server/scanner/LibraryItemScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0391 | job | scanner: `LibraryScan` | `server/scanner/LibraryScan.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0392 | job | scanner: `LibraryScanner` | `server/scanner/LibraryScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0393 | job | scanner: `MediaProbeData` | `server/scanner/MediaProbeData.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0394 | job | scanner: `NfoFileScanner` | `server/scanner/NfoFileScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0395 | job | scanner: `OpfFileScanner` | `server/scanner/OpfFileScanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0396 | job | scanner: `PodcastScanner` | `server/scanner/PodcastScanner.js` | P1 library | todo |  |
| A0397 | job | scanner: `ScanLogger` | `server/scanner/ScanLogger.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |
| A0398 | job | scanner: `Scanner` | `server/scanner/Scanner.js` | P1 library | done | `server/tests/parity_abs_scanner.rs::a_scan_writes_the_originals_rows` |

### ABS web

| ID | Kind | Item | Upstream source | Track | Status | Proof |
| --- | --- | --- | --- | --- | --- | --- |
| W0001 | page | `/account` | `client/pages/account.vue` | W web fork | todo |  |
| W0002 | page | `/config` | `client/pages/config.vue` | W web fork | todo |  |
| W0003 | page | `/` | `client/pages/index.vue` | W web fork | todo |  |
| W0004 | page | `/login` | `client/pages/login.vue` | W web fork | todo |  |
| W0005 | page | `/oops` | `client/pages/oops.vue` | W web fork | todo |  |
| W0006 | page | `/playlist/:id` | `client/pages/playlist/_id.vue` | W web fork | todo |  |
| W0007 | page | `/collection/:id` | `client/pages/collection/_id.vue` | W web fork | todo |  |
| W0008 | page | `/config/authentication` | `client/pages/config/authentication.vue` | W web fork | todo |  |
| W0009 | page | `/config/backups` | `client/pages/config/backups.vue` | W web fork | todo |  |
| W0010 | page | `/config/email` | `client/pages/config/email.vue` | W web fork | todo |  |
| W0011 | page | `/config` | `client/pages/config/index.vue` | W web fork | todo |  |
| W0012 | page | `/config/libraries` | `client/pages/config/libraries.vue` | W web fork | todo |  |
| W0013 | page | `/config/log` | `client/pages/config/log.vue` | W web fork | todo |  |
| W0014 | page | `/config/notifications` | `client/pages/config/notifications.vue` | W web fork | todo |  |
| W0015 | page | `/config/rss-feeds` | `client/pages/config/rss-feeds.vue` | W web fork | todo |  |
| W0016 | page | `/config/sessions` | `client/pages/config/sessions.vue` | W web fork | todo |  |
| W0017 | page | `/config/stats` | `client/pages/config/stats.vue` | W web fork | todo |  |
| W0018 | page | `/config/users` | `client/pages/config/users/index.vue` | W web fork | todo |  |
| W0019 | page | `/config/users/:id` | `client/pages/config/users/_id/index.vue` | W web fork | todo |  |
| W0020 | page | `/config/users/:id/sessions` | `client/pages/config/users/_id/sessions.vue` | W web fork | todo |  |
| W0021 | page | `/config/api-keys` | `client/pages/config/api-keys/index.vue` | W web fork | todo |  |
| W0022 | page | `/config/item-metadata-utils/custom-metadata-providers` | `client/pages/config/item-metadata-utils/custom-metadata-providers.vue` | W web fork | todo |  |
| W0023 | page | `/config/item-metadata-utils/genres` | `client/pages/config/item-metadata-utils/genres.vue` | W web fork | todo |  |
| W0024 | page | `/config/item-metadata-utils` | `client/pages/config/item-metadata-utils/index.vue` | W web fork | todo |  |
| W0025 | page | `/config/item-metadata-utils/tags` | `client/pages/config/item-metadata-utils/tags.vue` | W web fork | todo |  |
| W0026 | page | `/library/:library` | `client/pages/library/_library/index.vue` | W web fork | todo |  |
| W0027 | page | `/library/:library/narrators` | `client/pages/library/_library/narrators.vue` | W web fork | todo |  |
| W0028 | page | `/library/:library/search` | `client/pages/library/_library/search.vue` | W web fork | todo |  |
| W0029 | page | `/library/:library/stats` | `client/pages/library/_library/stats.vue` | W web fork | todo |  |
| W0030 | page | `/library/:library/series/:id` | `client/pages/library/_library/series/_id.vue` | W web fork | todo |  |
| W0031 | page | `/library/:library/podcast/download-queue` | `client/pages/library/_library/podcast/download-queue.vue` | W web fork | todo |  |
| W0032 | page | `/library/:library/podcast/latest` | `client/pages/library/_library/podcast/latest.vue` | W web fork | todo |  |
| W0033 | page | `/library/:library/podcast/search` | `client/pages/library/_library/podcast/search.vue` | W web fork | todo |  |
| W0034 | page | `/library/:library/bookshelf/:id` | `client/pages/library/_library/bookshelf/_id.vue` | W web fork | todo |  |
| W0035 | page | `/share/:slug` | `client/pages/share/_slug.vue` | W web fork | todo |  |
| W0036 | page | `/upload` | `client/pages/upload/index.vue` | W web fork | todo |  |
| W0037 | page | `/audiobook/:id/chapters` | `client/pages/audiobook/_id/chapters.vue` | W web fork | todo |  |
| W0038 | page | `/audiobook/:id/edit` | `client/pages/audiobook/_id/edit.vue` | W web fork | todo |  |
| W0039 | page | `/audiobook/:id/manage` | `client/pages/audiobook/_id/manage.vue` | W web fork | todo |  |
| W0040 | page | `/item/:id` | `client/pages/item/_id/index.vue` | W web fork | todo |  |
| W0041 | page | `/author/:id` | `client/pages/author/_id.vue` | W web fork | todo |  |
| W0042 | page | `/batch` | `client/pages/batch/index.vue` | W web fork | todo |  |
| W0043 | action | menu action `batch-mark-as-finished` | `client/components/tables/podcast/LazyEpisodesTable.vue#L105` | W web fork | todo |  |
| W0044 | action | menu action `bookmarks` | `client/pages/item/_id/index.vue#L388` | W web fork | todo |  |
| W0045 | action | menu action `collapse-series` | `client/components/app/BookShelfToolbar.vue#L355` | W web fork | todo |  |
| W0046 | action | menu action `collapse-sub-series` | `client/components/app/BookShelfToolbar.vue#L370` | W web fork | todo |  |
| W0047 | action | menu action `collections` | `client/pages/item/_id/index.vue#L374` | W web fork | todo |  |
| W0048 | action | menu action `create-playlist` | `client/pages/collection/_id.vue#L121` | W web fork | todo |  |
| W0049 | action | menu action `delete` | `client/components/tables/LibraryFilesTableRow.vue#L61` | W web fork | todo |  |
| W0050 | action | menu action `download` | `client/components/tables/LibraryFilesTableRow.vue#L55` | W web fork | todo |  |
| W0051 | action | menu action `edit` | `client/components/tables/library/LibraryItem.vue#L70` | W web fork | todo |  |
| W0052 | action | menu action `expand-series` | `client/components/app/BookShelfToolbar.vue#L350` | W web fork | todo |  |
| W0053 | action | menu action `expand-sub-series` | `client/components/app/BookShelfToolbar.vue#L365` | W web fork | todo |  |
| W0054 | action | menu action `export-opml` | `client/components/app/BookShelfToolbar.vue#L316` | W web fork | todo |  |
| W0055 | action | menu action `hide-subtitles` | `client/components/app/BookShelfToolbar.vue#L335` | W web fork | todo |  |
| W0056 | action | menu action `mark-series-finished` | `client/components/app/BookShelfToolbar.vue#L133` | W web fork | todo |  |
| W0057 | action | menu action `match-books` | `client/components/tables/library/LibraryItem.vue#L82` | W web fork | todo |  |
| W0058 | action | menu action `more` | `client/components/tables/LibraryFilesTableRow.vue#L68` | W web fork | todo |  |
| W0059 | action | menu action `open-rss-feed` | `client/components/app/BookShelfToolbar.vue#L140` | W web fork | todo |  |
| W0060 | action | menu action `playlists` | `client/pages/item/_id/index.vue#L381` | W web fork | todo |  |
| W0061 | action | menu action `quick-embed` | `client/components/app/Appbar.vue#L174` | W web fork | todo |  |
| W0062 | action | menu action `quick-match` | `client/components/app/Appbar.vue#L167` | W web fork | todo |  |
| W0063 | action | menu action `quick-match-episodes` | `client/components/tables/podcast/LazyEpisodesTable.vue#L100` | W web fork | todo |  |
| W0064 | action | menu action `re-add-to-continue-listening` | `client/components/app/BookShelfToolbar.vue#L147` | W web fork | todo |  |
| W0065 | action | menu action `rescan` | `client/components/app/Appbar.vue#L180` | W web fork | todo |  |
| W0066 | action | menu action `rss-feeds` | `client/pages/item/_id/index.vue#L395` | W web fork | todo |  |
| W0067 | action | menu action `scan` | `client/components/tables/library/LibraryItem.vue#L75` | W web fork | todo |  |
| W0068 | action | menu action `sendToDevice` | `client/pages/item/_id/index.vue#L412` | W web fork | todo |  |
| W0069 | action | menu action `share` | `client/pages/item/_id/index.vue#L422` | W web fork | todo |  |
| W0070 | action | menu action `show-subtitles` | `client/components/app/BookShelfToolbar.vue#L340` | W web fork | todo |  |
| W0071 | action | menu action `updateStatus` | `client/components/tables/EbookFilesTableRow.vue#L63` | W web fork | todo |  |

### ABS app

| ID | Kind | Item | Upstream source | Track | Status | Proof |
| --- | --- | --- | --- | --- | --- | --- |
| M0001 | page | `/account` | `pages/account.vue` | M mobile fork | done | `docs/parity/screens/M0001.png` (signed in to Folio: its address, the user and the server version) |
| M0002 | page | `/bookshelf` | `pages/bookshelf.vue` | M mobile fork | todo |  |
| M0003 | page | `/connect` | `pages/connect.vue` | M mobile fork | done | `docs/parity/screens/M0003.png` (connect and sign in to a Folio server, then its bookshelf) |
| M0004 | page | `/downloading` | `pages/downloading.vue` | M mobile fork | todo |  |
| M0005 | page | `/downloads` | `pages/downloads.vue` | M mobile fork | todo |  |
| M0006 | page | `/` | `pages/index.vue` | M mobile fork | todo |  |
| M0007 | page | `/logs` | `pages/logs.vue` | M mobile fork | done | `docs/parity/screens/M0007.png` (the app's own log after signing in to Folio) |
| M0008 | page | `/search` | `pages/search.vue` | M mobile fork | todo |  |
| M0009 | page | `/settings` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart), `docs/parity/screens/M0009.png` |
| M0010 | page | `/stats` | `pages/stats.vue` | M mobile fork | todo |  |
| M0011 | page | `/playlist/:id` | `pages/playlist/_id.vue` | M mobile fork | todo |  |
| M0012 | page | `/collection/:id` | `pages/collection/_id.vue` | M mobile fork | todo |  |
| M0013 | page | `/media/:id/history` | `pages/media/_id/history.vue` | M mobile fork | todo |  |
| M0014 | page | `/bookshelf/add-podcast` | `pages/bookshelf/add-podcast.vue` | M mobile fork | todo |  |
| M0015 | page | `/bookshelf/authors` | `pages/bookshelf/authors.vue` | M mobile fork | todo |  |
| M0016 | page | `/bookshelf/collections` | `pages/bookshelf/collections.vue` | M mobile fork | todo |  |
| M0017 | page | `/bookshelf` | `pages/bookshelf/index.vue` | M mobile fork | todo |  |
| M0018 | page | `/bookshelf/latest` | `pages/bookshelf/latest.vue` | M mobile fork | todo |  |
| M0019 | page | `/bookshelf/library` | `pages/bookshelf/library.vue` | M mobile fork | done | `docs/parity/screens/M0019.png` (Folio's library with the original's recorded books) |
| M0020 | page | `/bookshelf/playlists` | `pages/bookshelf/playlists.vue` | M mobile fork | todo |  |
| M0021 | page | `/bookshelf/series/:id` | `pages/bookshelf/series/_id.vue` | M mobile fork | todo |  |
| M0022 | page | `/bookshelf/series` | `pages/bookshelf/series/index.vue` | M mobile fork | todo |  |
| M0023 | page | `/item/:id` | `pages/item/_id/index.vue` | M mobile fork | done | `docs/parity/screens/M0023.png` (a book from Folio: author, series, narrator, genre, chapters, tracks) |
| M0024 | page | `/item/:id/:episode` | `pages/item/_id/_episode/index.vue` | M mobile fork | todo |  |
| M0025 | page | `/localMedia/folders/:id` | `pages/localMedia/folders/_id.vue` | M mobile fork | todo |  |
| M0026 | page | `/localMedia/folders` | `pages/localMedia/folders/index.vue` | M mobile fork | todo |  |
| M0027 | page | `/localMedia/item/:id` | `pages/localMedia/item/_id.vue` | M mobile fork | todo |  |
| M0028 | setting | device `allowSeekingOnMediaControls` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0029 | setting | device `androidAutoBrowseLimitForGrouping` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0030 | setting | device `androidAutoBrowseSeriesSequenceOrder` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0031 | setting | device `autoSleepTimer` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0032 | setting | device `autoSleepTimerAutoRewind` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0033 | setting | device `autoSleepTimerAutoRewindTime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0034 | setting | device `autoSleepTimerEndTime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0035 | setting | device `autoSleepTimerStartTime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0036 | setting | device `disableAutoRewind` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0037 | setting | device `disableShakeToResetSleepTimer` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0038 | setting | device `disableSleepTimerFadeOut` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0039 | setting | device `disableSleepTimerResetFeedback` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0040 | setting | device `downloadUsingCellular` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0041 | setting | device `enableAltView` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0042 | setting | device `enableMp` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0043 | setting | device `enableSleepTimerAlmostDoneChime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0044 | setting | device `hapticFeedback` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0045 | setting | device `jumpBackwardsTime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0046 | setting | device `jumpForwardTime` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0047 | setting | device `languageCode` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0048 | setting | device `lockOrientation` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0049 | setting | device `shakeSensitivity` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0050 | setting | device `sleepTimerLength` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0051 | setting | device `streamingUsingCellular` | `pages/settings.vue` | M mobile fork | done | `abs-app/folio/e2e/settings.cjs` (changed on the page, saved, kept after a restart) |
| M0052 | action | offline downloads and local media | `android/, plugins/` | M mobile fork | todo |  |
| M0053 | action | Android Auto browse tree | `android/, plugins/` | M mobile fork | todo |  |
| M0054 | action | media notification and lock-screen controls | `android/, plugins/` | M mobile fork | todo |  |
| M0055 | action | sleep timer with shake to reset | `android/, plugins/` | M mobile fork | todo |  |
| M0056 | action | Chromecast | `android/, plugins/` | M mobile fork | todo |  |
| M0057 | action | local progress sync with the server | `android/, plugins/` | M mobile fork | todo |  |
| M0058 | action | ebook reader (EPUB, PDF, comic) | `android/, plugins/` | M mobile fork | todo |  |

### Readarr

| ID | Kind | Item | Upstream source | Track | Status | Proof |
| --- | --- | --- | --- | --- | --- | --- |
| R0001 | endpoint | `GET /api` (ApiInfo) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0002 | endpoint | `POST /login` (Authentication) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0003 | endpoint | `GET /login` (StaticResource) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0004 | endpoint | `GET /logout` (Authentication) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0005 | endpoint | `GET /api/v1/author` (Author) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0006 | endpoint | `POST /api/v1/author` (Author) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0007 | endpoint | `PUT /api/v1/author/{id}` (Author) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | wip | checks and changes done (`server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches`); `moveFiles` and the refresh after a metadata profile change come with commands (R5) |
| R0008 | endpoint | `DELETE /api/v1/author/{id}` (Author) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0009 | endpoint | `GET /api/v1/author/{id}` (Author) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0010 | endpoint | `PUT /api/v1/author/editor` (AuthorEditor) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | wip | changes done (`server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches`); moving files to a new root folder comes with commands (R5) |
| R0011 | endpoint | `DELETE /api/v1/author/editor` (AuthorEditor) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0012 | endpoint | `GET /api/v1/author/lookup` (AuthorLookup) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0013 | endpoint | `GET /api/v1/system/backup` (Backup) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0014 | endpoint | `DELETE /api/v1/system/backup/{id}` (Backup) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0015 | endpoint | `POST /api/v1/system/backup/restore/{id}` (Backup) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0016 | endpoint | `POST /api/v1/system/backup/restore/upload` (Backup) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0017 | endpoint | `GET /api/v1/blocklist` (Blocklist) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0018 | endpoint | `DELETE /api/v1/blocklist/{id}` (Blocklist) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0019 | endpoint | `DELETE /api/v1/blocklist/bulk` (Blocklist) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0020 | endpoint | `GET /api/v1/book` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0021 | endpoint | `POST /api/v1/book` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0022 | endpoint | `GET /api/v1/book/{id}/overview` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0023 | endpoint | `PUT /api/v1/book/{id}` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0024 | endpoint | `DELETE /api/v1/book/{id}` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0025 | endpoint | `GET /api/v1/book/{id}` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0026 | endpoint | `PUT /api/v1/book/monitor` (Book) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0027 | endpoint | `PUT /api/v1/book/editor` (BookEditor) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0028 | endpoint | `DELETE /api/v1/book/editor` (BookEditor) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0029 | endpoint | `GET /api/v1/bookfile` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | wip | ebook files done (`server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches`); audio media info with the import |
| R0030 | endpoint | `PUT /api/v1/bookfile/{id}` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0031 | endpoint | `DELETE /api/v1/bookfile/{id}` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0032 | endpoint | `GET /api/v1/bookfile/{id}` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | wip | ebook files done (`server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches`); real files' tags with the import |
| R0033 | endpoint | `PUT /api/v1/bookfile/editor` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0034 | endpoint | `DELETE /api/v1/bookfile/bulk` (BookFile) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0035 | endpoint | `GET /api/v1/book/lookup` (BookLookup) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0036 | endpoint | `POST /api/v1/bookshelf` (Bookshelf) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0037 | endpoint | `GET /api/v1/calendar` (Calendar) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0038 | endpoint | `GET /api/v1/calendar/{id}` (Calendar) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0039 | endpoint | `GET /feed/v1/calendar/readarr.ics` (CalendarFeed) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0040 | endpoint | `POST /api/v1/command` (Command) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0041 | endpoint | `GET /api/v1/command` (Command) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0042 | endpoint | `DELETE /api/v1/command/{id}` (Command) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0043 | endpoint | `GET /api/v1/command/{id}` (Command) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0044 | endpoint | `GET /api/v1/customfilter` (CustomFilter) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0045 | endpoint | `POST /api/v1/customfilter` (CustomFilter) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0046 | endpoint | `PUT /api/v1/customfilter/{id}` (CustomFilter) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0047 | endpoint | `DELETE /api/v1/customfilter/{id}` (CustomFilter) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0048 | endpoint | `GET /api/v1/customfilter/{id}` (CustomFilter) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0049 | endpoint | `POST /api/v1/customformat` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0050 | endpoint | `GET /api/v1/customformat` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0051 | endpoint | `PUT /api/v1/customformat/{id}` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0052 | endpoint | `DELETE /api/v1/customformat/{id}` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0053 | endpoint | `GET /api/v1/customformat/{id}` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0054 | endpoint | `GET /api/v1/customformat/schema` (CustomFormat) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0055 | endpoint | `GET /api/v1/wanted/cutoff` (Cutoff) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0056 | endpoint | `GET /api/v1/wanted/cutoff/{id}` (Cutoff) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0057 | endpoint | `POST /api/v1/delayprofile` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0058 | endpoint | `GET /api/v1/delayprofile` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0059 | endpoint | `DELETE /api/v1/delayprofile/{id}` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0060 | endpoint | `PUT /api/v1/delayprofile/{id}` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0061 | endpoint | `GET /api/v1/delayprofile/{id}` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0062 | endpoint | `PUT /api/v1/delayprofile/reorder/{id}` (DelayProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0063 | endpoint | `GET /api/v1/config/development` (DevelopmentConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0064 | endpoint | `PUT /api/v1/config/development/{id}` (DevelopmentConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0065 | endpoint | `GET /api/v1/config/development/{id}` (DevelopmentConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0066 | endpoint | `GET /api/v1/diskspace` (DiskSpace) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0067 | endpoint | `GET /api/v1/downloadclient` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0068 | endpoint | `POST /api/v1/downloadclient` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0069 | endpoint | `PUT /api/v1/downloadclient/{id}` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0070 | endpoint | `DELETE /api/v1/downloadclient/{id}` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0071 | endpoint | `GET /api/v1/downloadclient/{id}` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0072 | endpoint | `PUT /api/v1/downloadclient/bulk` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0073 | endpoint | `DELETE /api/v1/downloadclient/bulk` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0074 | endpoint | `GET /api/v1/downloadclient/schema` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0075 | endpoint | `POST /api/v1/downloadclient/test` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0076 | endpoint | `POST /api/v1/downloadclient/testall` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0077 | endpoint | `POST /api/v1/downloadclient/action/{name}` (DownloadClient) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0078 | endpoint | `GET /api/v1/config/downloadclient` (DownloadClientConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0079 | endpoint | `PUT /api/v1/config/downloadclient/{id}` (DownloadClientConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0080 | endpoint | `GET /api/v1/config/downloadclient/{id}` (DownloadClientConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0081 | endpoint | `GET /api/v1/edition` (Edition) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0082 | endpoint | `GET /api/v1/filesystem` (FileSystem) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0083 | endpoint | `GET /api/v1/filesystem/type` (FileSystem) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0084 | endpoint | `GET /api/v1/filesystem/mediafiles` (FileSystem) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0085 | endpoint | `GET /api/v1/health` (Health) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0086 | endpoint | `GET /api/v1/history` (History) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0087 | endpoint | `GET /api/v1/history/since` (History) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0088 | endpoint | `GET /api/v1/history/author` (History) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0089 | endpoint | `POST /api/v1/history/failed/{id}` (History) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0090 | endpoint | `GET /api/v1/config/host` (HostConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0091 | endpoint | `PUT /api/v1/config/host/{id}` (HostConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0092 | endpoint | `GET /api/v1/config/host/{id}` (HostConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0093 | endpoint | `GET /api/v1/importlist` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0094 | endpoint | `POST /api/v1/importlist` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0095 | endpoint | `PUT /api/v1/importlist/{id}` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0096 | endpoint | `DELETE /api/v1/importlist/{id}` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0097 | endpoint | `GET /api/v1/importlist/{id}` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0098 | endpoint | `PUT /api/v1/importlist/bulk` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0099 | endpoint | `DELETE /api/v1/importlist/bulk` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0100 | endpoint | `GET /api/v1/importlist/schema` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0101 | endpoint | `POST /api/v1/importlist/test` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0102 | endpoint | `POST /api/v1/importlist/testall` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0103 | endpoint | `POST /api/v1/importlist/action/{name}` (ImportList) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0104 | endpoint | `GET /api/v1/importlistexclusion` (ImportListExclusion) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0105 | endpoint | `POST /api/v1/importlistexclusion` (ImportListExclusion) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0106 | endpoint | `PUT /api/v1/importlistexclusion/{id}` (ImportListExclusion) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0107 | endpoint | `DELETE /api/v1/importlistexclusion/{id}` (ImportListExclusion) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0108 | endpoint | `GET /api/v1/importlistexclusion/{id}` (ImportListExclusion) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0109 | endpoint | `GET /api/v1/indexer` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0110 | endpoint | `POST /api/v1/indexer` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0111 | endpoint | `PUT /api/v1/indexer/{id}` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0112 | endpoint | `DELETE /api/v1/indexer/{id}` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0113 | endpoint | `GET /api/v1/indexer/{id}` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0114 | endpoint | `PUT /api/v1/indexer/bulk` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0115 | endpoint | `DELETE /api/v1/indexer/bulk` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0116 | endpoint | `GET /api/v1/indexer/schema` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0117 | endpoint | `POST /api/v1/indexer/test` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0118 | endpoint | `POST /api/v1/indexer/testall` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0119 | endpoint | `POST /api/v1/indexer/action/{name}` (Indexer) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0120 | endpoint | `GET /api/v1/config/indexer` (IndexerConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0121 | endpoint | `PUT /api/v1/config/indexer/{id}` (IndexerConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0122 | endpoint | `GET /api/v1/config/indexer/{id}` (IndexerConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0123 | endpoint | `GET /api/v1/indexerflag` (IndexerFlag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0124 | endpoint | `GET /api/v1/language` (Language) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0125 | endpoint | `GET /api/v1/language/{id}` (Language) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0126 | endpoint | `GET /api/v1/localization` (Localization) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0127 | endpoint | `GET /api/v1/log` (Log) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0128 | endpoint | `GET /api/v1/log/file` (LogFile) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0129 | endpoint | `GET /api/v1/log/file/{filename}` (LogFile) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0130 | endpoint | `POST /api/v1/manualimport` (ManualImport) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0131 | endpoint | `GET /api/v1/manualimport` (ManualImport) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0132 | endpoint | `GET /api/v1/mediacover/author/{authorId}/{filename}` (MediaCover) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0133 | endpoint | `GET /api/v1/mediacover/book/{bookId}/{filename}` (MediaCover) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0134 | endpoint | `GET /api/v1/config/mediamanagement` (MediaManagementConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0135 | endpoint | `PUT /api/v1/config/mediamanagement/{id}` (MediaManagementConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0136 | endpoint | `GET /api/v1/config/mediamanagement/{id}` (MediaManagementConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0137 | endpoint | `GET /api/v1/metadata` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0138 | endpoint | `POST /api/v1/metadata` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0139 | endpoint | `PUT /api/v1/metadata/{id}` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0140 | endpoint | `DELETE /api/v1/metadata/{id}` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0141 | endpoint | `GET /api/v1/metadata/{id}` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0142 | endpoint | `GET /api/v1/metadata/schema` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0143 | endpoint | `POST /api/v1/metadata/test` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0144 | endpoint | `POST /api/v1/metadata/testall` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0145 | endpoint | `POST /api/v1/metadata/action/{name}` (Metadata) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0146 | endpoint | `POST /api/v1/metadataprofile` (MetadataProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0147 | endpoint | `GET /api/v1/metadataprofile` (MetadataProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0148 | endpoint | `DELETE /api/v1/metadataprofile/{id}` (MetadataProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0149 | endpoint | `PUT /api/v1/metadataprofile/{id}` (MetadataProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0150 | endpoint | `GET /api/v1/metadataprofile/{id}` (MetadataProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0151 | endpoint | `GET /api/v1/metadataprofile/schema` (MetadataProfileSchema) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0152 | endpoint | `GET /api/v1/config/metadataprovider` (MetadataProviderConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0153 | endpoint | `PUT /api/v1/config/metadataprovider/{id}` (MetadataProviderConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0154 | endpoint | `GET /api/v1/config/metadataprovider/{id}` (MetadataProviderConfig) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0155 | endpoint | `GET /api/v1/wanted/missing` (Missing) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0156 | endpoint | `GET /api/v1/wanted/missing/{id}` (Missing) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0157 | endpoint | `GET /api/v1/config/naming` (NamingConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0158 | endpoint | `PUT /api/v1/config/naming/{id}` (NamingConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0159 | endpoint | `GET /api/v1/config/naming/{id}` (NamingConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0160 | endpoint | `GET /api/v1/config/naming/examples` (NamingConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0161 | endpoint | `GET /api/v1/notification` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0162 | endpoint | `POST /api/v1/notification` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0163 | endpoint | `PUT /api/v1/notification/{id}` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0164 | endpoint | `DELETE /api/v1/notification/{id}` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0165 | endpoint | `GET /api/v1/notification/{id}` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0166 | endpoint | `GET /api/v1/notification/schema` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0167 | endpoint | `POST /api/v1/notification/test` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0168 | endpoint | `POST /api/v1/notification/testall` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0169 | endpoint | `POST /api/v1/notification/action/{name}` (Notification) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0170 | endpoint | `GET /api/v1/parse` (Parse) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0171 | endpoint | `GET /ping` (Ping) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0172 | endpoint | `HEAD /ping` (Ping) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0173 | endpoint | `PUT /api/v1/qualitydefinition/{id}` (QualityDefinition) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0174 | endpoint | `GET /api/v1/qualitydefinition/{id}` (QualityDefinition) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0175 | endpoint | `GET /api/v1/qualitydefinition` (QualityDefinition) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0176 | endpoint | `PUT /api/v1/qualitydefinition/update` (QualityDefinition) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0177 | endpoint | `POST /api/v1/qualityprofile` (QualityProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0178 | endpoint | `GET /api/v1/qualityprofile` (QualityProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0179 | endpoint | `DELETE /api/v1/qualityprofile/{id}` (QualityProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0180 | endpoint | `PUT /api/v1/qualityprofile/{id}` (QualityProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0181 | endpoint | `GET /api/v1/qualityprofile/{id}` (QualityProfile) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0182 | endpoint | `GET /api/v1/qualityprofile/schema` (QualityProfileSchema) | `src/Readarr.Api.V1/openapi.json` | R3 profiles | todo |  |
| R0183 | endpoint | `DELETE /api/v1/queue/{id}` (Queue) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0184 | endpoint | `DELETE /api/v1/queue/bulk` (Queue) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0185 | endpoint | `GET /api/v1/queue` (Queue) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0186 | endpoint | `POST /api/v1/queue/grab/{id}` (QueueAction) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0187 | endpoint | `POST /api/v1/queue/grab/bulk` (QueueAction) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0188 | endpoint | `GET /api/v1/queue/details` (QueueDetails) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0189 | endpoint | `GET /api/v1/queue/status` (QueueStatus) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0190 | endpoint | `POST /api/v1/release` (Release) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0191 | endpoint | `GET /api/v1/release` (Release) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0192 | endpoint | `GET /api/v1/releaseprofile` (ReleaseProfile) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0193 | endpoint | `POST /api/v1/releaseprofile` (ReleaseProfile) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0194 | endpoint | `PUT /api/v1/releaseprofile/{id}` (ReleaseProfile) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0195 | endpoint | `DELETE /api/v1/releaseprofile/{id}` (ReleaseProfile) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0196 | endpoint | `GET /api/v1/releaseprofile/{id}` (ReleaseProfile) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0197 | endpoint | `POST /api/v1/release/push` (ReleasePush) | `src/Readarr.Api.V1/openapi.json` | R2 wanted and downloads | todo |  |
| R0198 | endpoint | `POST /api/v1/remotepathmapping` (RemotePathMapping) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0199 | endpoint | `GET /api/v1/remotepathmapping` (RemotePathMapping) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0200 | endpoint | `DELETE /api/v1/remotepathmapping/{id}` (RemotePathMapping) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0201 | endpoint | `PUT /api/v1/remotepathmapping/{id}` (RemotePathMapping) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0202 | endpoint | `GET /api/v1/remotepathmapping/{id}` (RemotePathMapping) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0203 | endpoint | `GET /api/v1/rename` (RenameBook) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0204 | endpoint | `GET /api/v1/retag` (RetagBook) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0205 | endpoint | `POST /api/v1/rootfolder` (RootFolder) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0206 | endpoint | `GET /api/v1/rootfolder` (RootFolder) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0207 | endpoint | `PUT /api/v1/rootfolder/{id}` (RootFolder) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0208 | endpoint | `DELETE /api/v1/rootfolder/{id}` (RootFolder) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0209 | endpoint | `GET /api/v1/rootfolder/{id}` (RootFolder) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0210 | endpoint | `GET /api/v1/search` (Search) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | todo |  |
| R0211 | endpoint | `GET /api/v1/series` (Series) | `src/Readarr.Api.V1/openapi.json` | R1 authors and books | done | `server/tests/parity_readarr_books.rs::every_recorded_answer_of_authors_books_and_files_matches` |
| R0212 | endpoint | `GET /content/{path}` (StaticResource) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0213 | endpoint | `GET /` (StaticResource) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0214 | endpoint | `GET /{path}` (StaticResource) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0215 | endpoint | `GET /api/v1/system/status` (System) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0216 | endpoint | `GET /api/v1/system/routes` (System) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0217 | endpoint | `GET /api/v1/system/routes/duplicate` (System) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0218 | endpoint | `POST /api/v1/system/shutdown` (System) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0219 | endpoint | `POST /api/v1/system/restart` (System) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0220 | endpoint | `GET /api/v1/tag` (Tag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0221 | endpoint | `POST /api/v1/tag` (Tag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0222 | endpoint | `PUT /api/v1/tag/{id}` (Tag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0223 | endpoint | `DELETE /api/v1/tag/{id}` (Tag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0224 | endpoint | `GET /api/v1/tag/{id}` (Tag) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0225 | endpoint | `GET /api/v1/tag/detail` (TagDetails) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0226 | endpoint | `GET /api/v1/tag/detail/{id}` (TagDetails) | `src/Readarr.Api.V1/openapi.json` | R4 providers | todo |  |
| R0227 | endpoint | `GET /api/v1/system/task` (Task) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0228 | endpoint | `GET /api/v1/system/task/{id}` (Task) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0229 | endpoint | `PUT /api/v1/config/ui/{id}` (UiConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0230 | endpoint | `GET /api/v1/config/ui/{id}` (UiConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0231 | endpoint | `GET /api/v1/config/ui` (UiConfig) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0232 | endpoint | `GET /api/v1/update` (Update) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0233 | endpoint | `GET /api/v1/log/file/update` (UpdateLogFile) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0234 | endpoint | `GET /api/v1/log/file/update/{filename}` (UpdateLogFile) | `src/Readarr.Api.V1/openapi.json` | R5 system | todo |  |
| R0235 | setting | Development `MetadataSource` | `src/Readarr.Api.V1/Config/DevelopmentConfigResource.cs#L8` | R5 system | todo |  |
| R0236 | setting | Development `ConsoleLogLevel` | `src/Readarr.Api.V1/Config/DevelopmentConfigResource.cs#L9` | R5 system | todo |  |
| R0237 | setting | Development `LogSql` | `src/Readarr.Api.V1/Config/DevelopmentConfigResource.cs#L10` | R5 system | todo |  |
| R0238 | setting | Development `LogRotate` | `src/Readarr.Api.V1/Config/DevelopmentConfigResource.cs#L11` | R5 system | todo |  |
| R0239 | setting | Development `FilterSentryEvents` | `src/Readarr.Api.V1/Config/DevelopmentConfigResource.cs#L12` | R5 system | todo |  |
| R0240 | setting | DownloadClient `DownloadClientWorkingFolders` | `src/Readarr.Api.V1/Config/DownloadClientConfigResource.cs#L8` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0241 | setting | DownloadClient `EnableCompletedDownloadHandling` | `src/Readarr.Api.V1/Config/DownloadClientConfigResource.cs#L10` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0242 | setting | DownloadClient `AutoRedownloadFailed` | `src/Readarr.Api.V1/Config/DownloadClientConfigResource.cs#L11` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0243 | setting | DownloadClient `AutoRedownloadFailedFromInteractiveSearch` | `src/Readarr.Api.V1/Config/DownloadClientConfigResource.cs#L12` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0244 | setting | Host `BindAddress` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L12` | R5 system | todo |  |
| R0245 | setting | Host `Port` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L13` | R5 system | todo |  |
| R0246 | setting | Host `SslPort` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L14` | R5 system | todo |  |
| R0247 | setting | Host `EnableSsl` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L15` | R5 system | todo |  |
| R0248 | setting | Host `LaunchBrowser` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L16` | R5 system | todo |  |
| R0249 | setting | Host `AuthenticationMethod` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L17` | R5 system | todo |  |
| R0250 | setting | Host `AuthenticationRequired` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L18` | R5 system | todo |  |
| R0251 | setting | Host `AnalyticsEnabled` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L19` | R5 system | todo |  |
| R0252 | setting | Host `Username` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L20` | R5 system | todo |  |
| R0253 | setting | Host `Password` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L21` | R5 system | todo |  |
| R0254 | setting | Host `PasswordConfirmation` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L22` | R5 system | todo |  |
| R0255 | setting | Host `LogLevel` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L23` | R5 system | todo |  |
| R0256 | setting | Host `ConsoleLogLevel` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L24` | R5 system | todo |  |
| R0257 | setting | Host `Branch` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L25` | R5 system | todo |  |
| R0258 | setting | Host `ApiKey` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L26` | R5 system | todo |  |
| R0259 | setting | Host `SslCertPath` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L27` | R5 system | todo |  |
| R0260 | setting | Host `SslCertPassword` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L28` | R5 system | todo |  |
| R0261 | setting | Host `UrlBase` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L29` | R5 system | todo |  |
| R0262 | setting | Host `InstanceName` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L30` | R5 system | todo |  |
| R0263 | setting | Host `ApplicationUrl` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L31` | R5 system | todo |  |
| R0264 | setting | Host `UpdateAutomatically` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L32` | R5 system | todo |  |
| R0265 | setting | Host `UpdateMechanism` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L33` | R5 system | todo |  |
| R0266 | setting | Host `UpdateScriptPath` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L34` | R5 system | todo |  |
| R0267 | setting | Host `ProxyEnabled` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L35` | R5 system | todo |  |
| R0268 | setting | Host `ProxyType` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L36` | R5 system | todo |  |
| R0269 | setting | Host `ProxyHostname` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L37` | R5 system | todo |  |
| R0270 | setting | Host `ProxyPort` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L38` | R5 system | todo |  |
| R0271 | setting | Host `ProxyUsername` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L39` | R5 system | todo |  |
| R0272 | setting | Host `ProxyPassword` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L40` | R5 system | todo |  |
| R0273 | setting | Host `ProxyBypassFilter` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L41` | R5 system | todo |  |
| R0274 | setting | Host `ProxyBypassLocalAddresses` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L42` | R5 system | todo |  |
| R0275 | setting | Host `CertificateValidation` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L43` | R5 system | todo |  |
| R0276 | setting | Host `BackupFolder` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L44` | R5 system | todo |  |
| R0277 | setting | Host `BackupInterval` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L45` | R5 system | todo |  |
| R0278 | setting | Host `BackupRetention` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L46` | R5 system | todo |  |
| R0279 | setting | Host `TrustCgnatIpAddresses` | `src/Readarr.Api.V1/Config/HostConfigResource.cs#L47` | R5 system | todo |  |
| R0280 | setting | Indexer `MinimumAge` | `src/Readarr.Api.V1/Config/IndexerConfigResource.cs#L8` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0281 | setting | Indexer `MaximumSize` | `src/Readarr.Api.V1/Config/IndexerConfigResource.cs#L9` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0282 | setting | Indexer `Retention` | `src/Readarr.Api.V1/Config/IndexerConfigResource.cs#L10` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0283 | setting | Indexer `RssSyncInterval` | `src/Readarr.Api.V1/Config/IndexerConfigResource.cs#L11` | R5 system | n/a | Lantern replaces it (GOALS item 2) |
| R0284 | setting | MediaManagement `AutoUnmonitorPreviouslyDownloadedBooks` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L10` | R5 system | todo |  |
| R0285 | setting | MediaManagement `RecycleBin` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L11` | R5 system | todo |  |
| R0286 | setting | MediaManagement `RecycleBinCleanupDays` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L12` | R5 system | todo |  |
| R0287 | setting | MediaManagement `DownloadPropersAndRepacks` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L13` | R5 system | todo |  |
| R0288 | setting | MediaManagement `CreateEmptyAuthorFolders` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L14` | R5 system | todo |  |
| R0289 | setting | MediaManagement `DeleteEmptyFolders` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L15` | R5 system | todo |  |
| R0290 | setting | MediaManagement `FileDate` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L16` | R5 system | todo |  |
| R0291 | setting | MediaManagement `WatchLibraryForChanges` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L17` | R5 system | todo |  |
| R0292 | setting | MediaManagement `RescanAfterRefresh` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L18` | R5 system | todo |  |
| R0293 | setting | MediaManagement `AllowFingerprinting` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L19` | R5 system | todo |  |
| R0294 | setting | MediaManagement `SetPermissionsLinux` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L21` | R5 system | todo |  |
| R0295 | setting | MediaManagement `ChmodFolder` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L22` | R5 system | todo |  |
| R0296 | setting | MediaManagement `ChownGroup` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L23` | R5 system | todo |  |
| R0297 | setting | MediaManagement `SkipFreeSpaceCheckWhenImporting` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L25` | R5 system | todo |  |
| R0298 | setting | MediaManagement `MinimumFreeSpaceWhenImporting` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L26` | R5 system | todo |  |
| R0299 | setting | MediaManagement `CopyUsingHardlinks` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L27` | R5 system | todo |  |
| R0300 | setting | MediaManagement `ImportExtraFiles` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L28` | R5 system | todo |  |
| R0301 | setting | MediaManagement `ExtraFileExtensions` | `src/Readarr.Api.V1/Config/MediaManagementConfigResource.cs#L29` | R5 system | todo |  |
| R0302 | setting | MetadataProvider `WriteAudioTags` | `src/Readarr.Api.V1/Config/MetadataProviderConfigResource.cs#L8` | R5 system | todo |  |
| R0303 | setting | MetadataProvider `ScrubAudioTags` | `src/Readarr.Api.V1/Config/MetadataProviderConfigResource.cs#L9` | R5 system | todo |  |
| R0304 | setting | MetadataProvider `WriteBookTags` | `src/Readarr.Api.V1/Config/MetadataProviderConfigResource.cs#L10` | R5 system | todo |  |
| R0305 | setting | MetadataProvider `UpdateCovers` | `src/Readarr.Api.V1/Config/MetadataProviderConfigResource.cs#L11` | R5 system | todo |  |
| R0306 | setting | MetadataProvider `EmbedMetadata` | `src/Readarr.Api.V1/Config/MetadataProviderConfigResource.cs#L12` | R5 system | todo |  |
| R0307 | setting | Naming `RenameBooks` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L7` | R5 system | todo |  |
| R0308 | setting | Naming `ReplaceIllegalCharacters` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L8` | R5 system | todo |  |
| R0309 | setting | Naming `ColonReplacementFormat` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L9` | R5 system | todo |  |
| R0310 | setting | Naming `StandardBookFormat` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L10` | R5 system | todo |  |
| R0311 | setting | Naming `AuthorFolderFormat` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L11` | R5 system | todo |  |
| R0312 | setting | Naming `IncludeAuthorName` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L12` | R5 system | todo |  |
| R0313 | setting | Naming `IncludeBookTitle` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L13` | R5 system | todo |  |
| R0314 | setting | Naming `IncludeQuality` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L14` | R5 system | todo |  |
| R0315 | setting | Naming `ReplaceSpaces` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L15` | R5 system | todo |  |
| R0316 | setting | Naming `Separator` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L16` | R5 system | todo |  |
| R0317 | setting | Naming `NumberStyle` | `src/Readarr.Api.V1/Config/NamingConfigResource.cs#L17` | R5 system | todo |  |
| R0318 | setting | Ui `FirstDayOfWeek` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L9` | R5 system | todo |  |
| R0319 | setting | Ui `CalendarWeekColumnHeader` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L10` | R5 system | todo |  |
| R0320 | setting | Ui `ShortDateFormat` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L13` | R5 system | todo |  |
| R0321 | setting | Ui `LongDateFormat` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L14` | R5 system | todo |  |
| R0322 | setting | Ui `TimeFormat` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L15` | R5 system | todo |  |
| R0323 | setting | Ui `ShowRelativeDates` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L16` | R5 system | todo |  |
| R0324 | setting | Ui `EnableColorImpairedMode` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L18` | R5 system | todo |  |
| R0325 | setting | Ui `UILanguage` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L19` | R5 system | todo |  |
| R0326 | setting | Ui `Theme` | `src/Readarr.Api.V1/Config/UiConfigResource.cs#L20` | R5 system | todo |  |
| R0327 | job | scheduled `RefreshMonitoredDownloadsCommand` (every 1) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0328 | job | scheduled `MessagingCleanupCommand` (every 5) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0329 | job | scheduled `ApplicationUpdateCheckCommand` (every 6 * 60) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0330 | job | scheduled `CheckHealthCommand` (every 6 * 60) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0331 | job | scheduled `RefreshAuthorCommand` (every 24 * 60) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0332 | job | scheduled `RescanFoldersCommand` (every 24 * 60) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0333 | job | scheduled `HousekeepingCommand` (every 24 * 60) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0334 | job | scheduled `BackupCommand` (every GetBackupInterval()) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0335 | job | scheduled `ImportListSyncCommand` (every 5) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0336 | job | scheduled `RssSyncCommand` (every GetRssSyncInterval()) | `src/NzbDrone.Core/Jobs/TaskManager.cs` | R5 system | todo |  |
| R0337 | action | command `ResetQualityDefinitionsCommand` | `src/NzbDrone.Core/Qualities/Commands/ResetQualityDefinitionsCommand.cs` | R5 system | todo |  |
| R0338 | action | command `MessagingCleanupCommand` | `src/NzbDrone.Core/Messaging/Commands/MessagingCleanupCommand.cs` | R5 system | todo |  |
| R0339 | action | command `TestCommand` | `src/NzbDrone.Core/Messaging/Commands/TestCommand.cs` | R5 system | todo |  |
| R0340 | action | command `UnknownCommand` | `src/NzbDrone.Core/Messaging/Commands/UnknownCommand.cs` | R5 system | todo |  |
| R0341 | action | command `CleanUpRecycleBinCommand` | `src/NzbDrone.Core/MediaFiles/Commands/CleanUpRecycleBinCommand.cs` | R5 system | todo |  |
| R0342 | action | command `DownloadedBooksScanCommand` | `src/NzbDrone.Core/MediaFiles/Commands/DownloadedBooksScanCommand.cs` | R2 wanted and downloads | todo |  |
| R0343 | action | command `RenameAuthorCommand` | `src/NzbDrone.Core/MediaFiles/Commands/RenameAuthorCommand.cs` | R5 system | todo |  |
| R0344 | action | command `RenameFilesCommand` | `src/NzbDrone.Core/MediaFiles/Commands/RenameFilesCommand.cs` | R5 system | todo |  |
| R0345 | action | command `RescanFoldersCommand` | `src/NzbDrone.Core/MediaFiles/Commands/RescanFoldersCommand.cs` | R5 system | todo |  |
| R0346 | action | command `RetagAuthorCommand` | `src/NzbDrone.Core/MediaFiles/Commands/RetagAuthorCommand.cs` | R5 system | todo |  |
| R0347 | action | command `RetagFilesCommand` | `src/NzbDrone.Core/MediaFiles/Commands/RetagFilesCommand.cs` | R5 system | todo |  |
| R0348 | action | command `ManualImportCommand` | `src/NzbDrone.Core/MediaFiles/BookImport/Manual/ManualImportCommand.cs` | R5 system | todo |  |
| R0349 | action | command `AuthorSearchCommand` | `src/NzbDrone.Core/IndexerSearch/AuthorSearchCommand.cs` | R2 wanted and downloads | todo |  |
| R0350 | action | command `BookSearchCommand` | `src/NzbDrone.Core/IndexerSearch/BookSearchCommand.cs` | R2 wanted and downloads | todo |  |
| R0351 | action | command `CutoffUnmetBookSearchCommand` | `src/NzbDrone.Core/IndexerSearch/CutoffUnmetBookSearchCommand.cs` | R2 wanted and downloads | todo |  |
| R0352 | action | command `MissingBookSearchCommand` | `src/NzbDrone.Core/IndexerSearch/MissingBookSearchCommand.cs` | R2 wanted and downloads | todo |  |
| R0353 | action | command `ClearBlocklistCommand` | `src/NzbDrone.Core/Blocklisting/ClearBlocklistCommand.cs` | R5 system | todo |  |
| R0354 | action | command `RssSyncCommand` | `src/NzbDrone.Core/Indexers/RssSyncCommand.cs` | R2 wanted and downloads | todo |  |
| R0355 | action | command `ApplicationUpdateCheckCommand` | `src/NzbDrone.Core/Update/Commands/ApplicationUpdateCheckCommand.cs` | R5 system | todo |  |
| R0356 | action | command `ApplicationUpdateCommand` | `src/NzbDrone.Core/Update/Commands/ApplicationUpdateCommand.cs` | R5 system | todo |  |
| R0357 | action | command `CheckForFinishedDownloadCommand` | `src/NzbDrone.Core/Download/CheckForFinishedDownloadCommand.cs` | R2 wanted and downloads | todo |  |
| R0358 | action | command `ProcessMonitoredDownloadsCommand` | `src/NzbDrone.Core/Download/ProcessMonitoredDownloadsCommand.cs` | R2 wanted and downloads | todo |  |
| R0359 | action | command `RefreshMonitoredDownloadsCommand` | `src/NzbDrone.Core/Download/RefreshMonitoredDownloadsCommand.cs` | R2 wanted and downloads | todo |  |
| R0360 | action | command `ImportListSyncCommand` | `src/NzbDrone.Core/ImportLists/ImportListSyncCommand.cs` | R5 system | todo |  |
| R0361 | action | command `ResetApiKeyCommand` | `src/NzbDrone.Core/Configuration/ResetApiKeyCommand.cs` | R5 system | todo |  |
| R0362 | action | command `RestartCommand` | `src/NzbDrone.Core/Lifecycle/Commands/RestartCommand.cs` | R5 system | todo |  |
| R0363 | action | command `ShutdownCommand` | `src/NzbDrone.Core/Lifecycle/Commands/ShutdownCommand.cs` | R5 system | todo |  |
| R0364 | action | command `BulkMoveAuthorCommand` | `src/NzbDrone.Core/Books/Commands/BulkMoveAuthorCommand.cs` | R5 system | todo |  |
| R0365 | action | command `BulkRefreshAuthorCommand` | `src/NzbDrone.Core/Books/Commands/BulkRefreshAuthorCommand.cs` | R5 system | todo |  |
| R0366 | action | command `BulkRefreshBookCommand` | `src/NzbDrone.Core/Books/Commands/BulkRefreshBookCommand.cs` | R5 system | todo |  |
| R0367 | action | command `MoveAuthorCommand` | `src/NzbDrone.Core/Books/Commands/MoveAuthorCommand.cs` | R5 system | todo |  |
| R0368 | action | command `RefreshAuthorCommand` | `src/NzbDrone.Core/Books/Commands/RefreshAuthorCommand.cs` | R5 system | todo |  |
| R0369 | action | command `RefreshBookCommand` | `src/NzbDrone.Core/Books/Commands/RefreshBookCommand.cs` | R5 system | todo |  |
| R0370 | action | command `BackupCommand` | `src/NzbDrone.Core/Backup/BackupCommand.cs` | R5 system | todo |  |
| R0371 | action | command `ClearLogCommand` | `src/NzbDrone.Core/Instrumentation/Commands/ClearLogCommand.cs` | R5 system | todo |  |
| R0372 | action | command `DeleteLogFilesCommand` | `src/NzbDrone.Core/Instrumentation/Commands/DeleteLogFilesCommand.cs` | R5 system | todo |  |
| R0373 | action | command `DeleteUpdateLogFilesCommand` | `src/NzbDrone.Core/Instrumentation/Commands/DeleteUpdateLogFilesCommand.cs` | R5 system | todo |  |
| R0374 | action | command `HousekeepingCommand` | `src/NzbDrone.Core/Housekeeping/HousekeepingCommand.cs` | R5 system | todo |  |
| R0375 | action | command `CheckHealthCommand` | `src/NzbDrone.Core/HealthCheck/CheckHealthCommand.cs` | R5 system | todo |  |
| R0376 | provider | indexer `Exceptions` | `src/NzbDrone.Core/Indexers/Exceptions` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0377 | provider | indexer `FileList` | `src/NzbDrone.Core/Indexers/FileList` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0378 | provider | indexer `Gazelle` | `src/NzbDrone.Core/Indexers/Gazelle` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0379 | provider | indexer `IPTorrents` | `src/NzbDrone.Core/Indexers/IPTorrents` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0380 | provider | indexer `Newznab` | `src/NzbDrone.Core/Indexers/Newznab` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0381 | provider | indexer `Nyaa` | `src/NzbDrone.Core/Indexers/Nyaa` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0382 | provider | indexer `TorrentRss` | `src/NzbDrone.Core/Indexers/TorrentRss` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0383 | provider | indexer `Torrentleech` | `src/NzbDrone.Core/Indexers/Torrentleech` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0384 | provider | indexer `Torznab` | `src/NzbDrone.Core/Indexers/Torznab` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0385 | provider | download client `Aria2` | `src/NzbDrone.Core/Download/Clients/Aria2` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0386 | provider | download client `Blackhole` | `src/NzbDrone.Core/Download/Clients/Blackhole` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0387 | provider | download client `Deluge` | `src/NzbDrone.Core/Download/Clients/Deluge` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0388 | provider | download client `DownloadStation` | `src/NzbDrone.Core/Download/Clients/DownloadStation` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0389 | provider | download client `Flood` | `src/NzbDrone.Core/Download/Clients/Flood` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0390 | provider | download client `Hadouken` | `src/NzbDrone.Core/Download/Clients/Hadouken` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0391 | provider | download client `NzbVortex` | `src/NzbDrone.Core/Download/Clients/NzbVortex` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0392 | provider | download client `Nzbget` | `src/NzbDrone.Core/Download/Clients/Nzbget` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0393 | provider | download client `Pneumatic` | `src/NzbDrone.Core/Download/Clients/Pneumatic` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0394 | provider | download client `QBittorrent` | `src/NzbDrone.Core/Download/Clients/QBittorrent` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0395 | provider | download client `Sabnzbd` | `src/NzbDrone.Core/Download/Clients/Sabnzbd` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0396 | provider | download client `Transmission` | `src/NzbDrone.Core/Download/Clients/Transmission` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0397 | provider | download client `Vuze` | `src/NzbDrone.Core/Download/Clients/Vuze` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0398 | provider | download client `rTorrent` | `src/NzbDrone.Core/Download/Clients/rTorrent` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0399 | provider | download client `uTorrent` | `src/NzbDrone.Core/Download/Clients/uTorrent` | R4 providers | n/a | Lantern replaces it (GOALS item 2) |
| R0400 | provider | import list `Exceptions` | `src/NzbDrone.Core/ImportLists/Exceptions` | R4 providers | todo |  |
| R0401 | provider | import list `Exclusions` | `src/NzbDrone.Core/ImportLists/Exclusions` | R4 providers | todo |  |
| R0402 | provider | import list `Goodreads` | `src/NzbDrone.Core/ImportLists/Goodreads` | R4 providers | todo |  |
| R0403 | provider | import list `LazyLibrarian` | `src/NzbDrone.Core/ImportLists/LazyLibrarian` | R4 providers | todo |  |
| R0404 | provider | import list `Readarr` | `src/NzbDrone.Core/ImportLists/Readarr` | R4 providers | todo |  |
| R0405 | provider | notification `Apprise` | `src/NzbDrone.Core/Notifications/Apprise` | R4 providers | todo |  |
| R0406 | provider | notification `CustomScript` | `src/NzbDrone.Core/Notifications/CustomScript` | R4 providers | todo |  |
| R0407 | provider | notification `Discord` | `src/NzbDrone.Core/Notifications/Discord` | R4 providers | todo |  |
| R0408 | provider | notification `Email` | `src/NzbDrone.Core/Notifications/Email` | R4 providers | todo |  |
| R0409 | provider | notification `Goodreads` | `src/NzbDrone.Core/Notifications/Goodreads` | R4 providers | todo |  |
| R0410 | provider | notification `Gotify` | `src/NzbDrone.Core/Notifications/Gotify` | R4 providers | todo |  |
| R0411 | provider | notification `Join` | `src/NzbDrone.Core/Notifications/Join` | R4 providers | todo |  |
| R0412 | provider | notification `Kavita` | `src/NzbDrone.Core/Notifications/Kavita` | R4 providers | todo |  |
| R0413 | provider | notification `Mailgun` | `src/NzbDrone.Core/Notifications/Mailgun` | R4 providers | todo |  |
| R0414 | provider | notification `Notifiarr` | `src/NzbDrone.Core/Notifications/Notifiarr` | R4 providers | todo |  |
| R0415 | provider | notification `Ntfy` | `src/NzbDrone.Core/Notifications/Ntfy` | R4 providers | todo |  |
| R0416 | provider | notification `Plex` | `src/NzbDrone.Core/Notifications/Plex` | R4 providers | todo |  |
| R0417 | provider | notification `Prowl` | `src/NzbDrone.Core/Notifications/Prowl` | R4 providers | todo |  |
| R0418 | provider | notification `PushBullet` | `src/NzbDrone.Core/Notifications/PushBullet` | R4 providers | todo |  |
| R0419 | provider | notification `Pushover` | `src/NzbDrone.Core/Notifications/Pushover` | R4 providers | todo |  |
| R0420 | provider | notification `SendGrid` | `src/NzbDrone.Core/Notifications/SendGrid` | R4 providers | todo |  |
| R0421 | provider | notification `Signal` | `src/NzbDrone.Core/Notifications/Signal` | R4 providers | todo |  |
| R0422 | provider | notification `Simplepush` | `src/NzbDrone.Core/Notifications/Simplepush` | R4 providers | todo |  |
| R0423 | provider | notification `Slack` | `src/NzbDrone.Core/Notifications/Slack` | R4 providers | todo |  |
| R0424 | provider | notification `Subsonic` | `src/NzbDrone.Core/Notifications/Subsonic` | R4 providers | todo |  |
| R0425 | provider | notification `Synology` | `src/NzbDrone.Core/Notifications/Synology` | R4 providers | todo |  |
| R0426 | provider | notification `Telegram` | `src/NzbDrone.Core/Notifications/Telegram` | R4 providers | todo |  |
| R0427 | provider | notification `Twitter` | `src/NzbDrone.Core/Notifications/Twitter` | R4 providers | todo |  |
| R0428 | provider | notification `Webhook` | `src/NzbDrone.Core/Notifications/Webhook` | R4 providers | todo |  |
| R0429 | page | `/` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0430 | page | `/authors` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0431 | page | `/add/search` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0432 | page | `/shelf` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0433 | page | `/books` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0434 | page | `/unmapped` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0435 | page | `/author/:titleSlug` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0436 | page | `/book/:titleSlug` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0437 | page | `/calendar` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0438 | page | `/activity/history` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0439 | page | `/activity/queue` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0440 | page | `/activity/blocklist` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0441 | page | `/wanted/missing` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0442 | page | `/wanted/cutoffunmet` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0443 | page | `/settings` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0444 | page | `/settings/mediamanagement` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0445 | page | `/settings/profiles` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0446 | page | `/settings/quality` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0447 | page | `/settings/customformats` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0448 | page | `/settings/indexers` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0449 | page | `/settings/downloadclients` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0450 | page | `/settings/importlists` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0451 | page | `/settings/connect` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0452 | page | `/settings/metadata` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0453 | page | `/settings/tags` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0454 | page | `/settings/general` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0455 | page | `/settings/ui` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0456 | page | `/settings/development` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0457 | page | `/system/status` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0458 | page | `/system/tasks` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0459 | page | `/system/backup` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0460 | page | `/system/updates` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0461 | page | `/system/events` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
| R0462 | page | `/system/logs/files` | `frontend/src/App/AppRoutes.js` | F own pages | todo |  |
