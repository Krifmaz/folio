-- The test library for recording Readarr (and for the parity tests, which load the same rows into Folio).
-- Readarr's metadata server is gone, so authors and books can't be added through its API: they are written
-- straight into its database, in the formats Readarr itself writes (JSON documents with camelCase keys and
-- string enums, times as `yyyy-MM-dd HH:mm:ss.fffffffZ`). Ids are given, so Folio and Readarr agree on them.
-- The root folder (/books, id 1) and the tag `kids` (id 1) are made through the API before this runs.

INSERT INTO AuthorMetadata (Id, ForeignAuthorId, TitleSlug, Name, Overview, Disambiguation, Gender, Hometown, Born, Died, Status, Images, Links, Genres, Ratings, Aliases, SortName, NameLastFirst, SortNameLastFirst) VALUES
 (1, '1654', '1654', 'Terry Pratchett', 'Sir Terry Pratchett wrote the Discworld novels.', NULL, 'male', 'Beaconsfield', '1948-04-28 00:00:00Z', '2015-03-12 00:00:00Z', 1,
  '[{"url":"https://images.gr-assets.com/authors/1235562205p7/1654.jpg","coverType":"poster","extension":".jpg"}]',
  '[{"url":"https://www.goodreads.com/author/show/1654.Terry_Pratchett","name":"Goodreads"}]',
  '["fantasy","humor"]', '{"votes":1500000,"value":4.13}', '[]', 'terry pratchett', 'Pratchett, Terry', 'pratchett, terry'),
 (2, '874602', '874602', 'Ursula K. Le Guin', 'Ursula Kroeber Le Guin wrote Earthsea.', NULL, NULL, NULL, '1929-10-21 00:00:00Z', NULL, 0,
  '[]', '[]', '[]', '{"votes":0,"value":0}', '[]', 'ursula k. le guin', 'Le Guin, Ursula K.', 'le guin, ursula k.');

INSERT INTO Authors (Id, CleanName, Path, Monitored, LastInfoSync, QualityProfileId, Tags, Added, AddOptions, MetadataProfileId, AuthorMetadataId, MonitorNewItems) VALUES
 (1, 'terrypratchett', '/books/Terry Pratchett', 1, '2025-06-01 10:00:00Z', 1, '[1]', '2025-05-01 09:30:00Z',
  '{"booksToMonitor":[],"monitored":true,"searchForMissingBooks":false,"monitor":"all"}', 1, 1, 0),
 (2, 'ursulakleguin', '/books/Ursula K. Le Guin', 0, NULL, 2, '[]', '2025-05-02 09:30:00Z', NULL, 2, 2, 1);

INSERT INTO Books (Id, AuthorMetadataId, ForeignBookId, TitleSlug, Title, ReleaseDate, Links, Genres, Ratings, CleanTitle, Monitored, AnyEditionOk, LastInfoSync, Added, AddOptions, RelatedBooks, LastSearchTime) VALUES
 (1, 1, '64216', '64216', 'Guards! Guards!', '1989-11-01 00:00:00Z', '[{"url":"https://www.goodreads.com/work/editions/64216","name":"Goodreads Editions"}]', '["fantasy"]', '{"votes":200000,"value":4.32}', 'guardsguards', 1, 1, '2025-06-01 10:00:00Z', '2025-05-01 09:30:00Z', '{"addType":"automatic","searchForNewBook":false}', '[]', NULL),
 (2, 1, '1215958', '1215958', 'Small Gods', '1992-05-01 00:00:00Z', '[]', '["fantasy","religion"]', '{"votes":100000,"value":4.29}', 'smallgods', 1, 1, NULL, '2025-05-01 09:30:00Z', NULL, '[]', '2025-06-02 08:00:00Z'),
 (3, 1, '9000001', '9000001', 'A Book Not Out Yet', '2099-01-01 00:00:00Z', '[]', '[]', '{"votes":0,"value":0}', 'abooknotoutyet', 1, 1, NULL, '2025-05-01 09:30:00Z', NULL, '[]', NULL),
 (4, 2, '1561478', '1561478', 'A Wizard of Earthsea', '1968-01-01 00:00:00Z', '[]', '["fantasy"]', '{"votes":300000,"value":4.0}', 'awizardofearthsea', 0, 0, NULL, '2025-05-02 09:30:00Z', NULL, '[]', NULL);

INSERT INTO Editions (Id, BookId, ForeignEditionId, Isbn13, Asin, Title, TitleSlug, Language, Overview, Format, IsEbook, Disambiguation, Publisher, PageCount, ReleaseDate, Images, Links, Ratings, Monitored, ManualAdd) VALUES
 (1, 1, '64216-e1', '9780062237378', NULL, 'Guards! Guards!', '64216-e1', 'eng', 'The eighth Discworld novel.', 'Kindle Edition', 1, NULL, 'Harper', 416, '2013-08-27 00:00:00Z',
  '[{"url":"https://images.gr-assets.com/books/1/64216.jpg","coverType":"cover","extension":".jpg"}]', '[{"url":"https://www.goodreads.com/book/show/64216","name":"Goodreads Book"}]', '{"votes":1000,"value":4.32}', 1, 0),
 (2, 1, '64216-a1', NULL, 'B00XQKXNR8', 'Guards! Guards!', '64216-a1', 'eng', NULL, 'Audible Audio', 0, NULL, 'Isis', 0, '2015-01-01 00:00:00Z', '[]', '[]', '{"votes":10,"value":4.5}', 0, 0),
 (3, 2, '1215958-e1', '9780062237378', NULL, 'Small Gods', '1215958-e1', 'eng', NULL, 'Paperback', 0, NULL, 'Harper', 400, '2013-01-01 00:00:00Z', '[]', '[]', '{"votes":0,"value":0}', 1, 0),
 (4, 3, '9000001-e1', NULL, NULL, 'A Book Not Out Yet', '9000001-e1', 'eng', NULL, 'Hardcover', 0, NULL, NULL, 0, '2099-01-01 00:00:00Z', '[]', '[]', '{"votes":0,"value":0}', 1, 0),
 (5, 4, '1561478-e1', '9780547773742', NULL, 'A Wizard of Earthsea', '1561478-e1', 'eng', NULL, 'Paperback', 0, NULL, 'HMH', 183, '2012-09-11 00:00:00Z', '[]', '[]', '{"votes":0,"value":0}', 1, 0);

INSERT INTO BookFiles (Id, EditionId, CalibreId, Quality, Size, SceneName, DateAdded, ReleaseGroup, MediaInfo, Modified, Path, Part, OriginalFilePath, IndexerFlags) VALUES
 (1, 1, 0, '{"quality":3,"revision":{"version":1,"real":0,"isRepack":false}}', 2048, NULL, '2025-05-03 12:00:00Z', NULL, NULL, '2025-05-03 11:00:00Z', '/books/Terry Pratchett/Guards! Guards! (1989)/Guards! Guards! - Terry Pratchett.epub', 1, NULL, 0),
 (2, 0, 0, '{"quality":3,"revision":{"version":1,"real":0,"isRepack":false}}', 1024, NULL, '2025-05-04 12:00:00Z', NULL, NULL, '2025-05-04 11:00:00Z', '/books/Unsorted/Mystery Book.epub', 1, NULL, 0);

INSERT INTO Series (Id, ForeignSeriesId, Title, Description, Numbered, WorkCount, PrimaryWorkCount) VALUES
 (1, '40650', 'Discworld', 'The Discworld novels.', 1, 41, 41);

INSERT INTO SeriesBookLink (Id, SeriesId, BookId, Position, IsPrimary, SeriesPosition) VALUES
 (1, 1, 1, '8', 1, 8),
 (2, 1, 2, '13', 1, 13);
