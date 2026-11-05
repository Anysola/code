-- Shapes 0.5.1 reads but never writes itself, applied to the fork session of the rich fixture: parts moved to the
-- session_part overlay, pre-v2.1 names, a media blob ref, an interrupted last step and a running compaction.
CREATE TEMP TABLE target AS SELECT id AS session FROM session_v2 WHERE fork_session_id IS NOT NULL;
CREATE TEMP TABLE steps AS
  SELECT id, seq, row_number() OVER (ORDER BY seq) AS n, count(*) OVER () AS total FROM session_message
  WHERE session_id = (SELECT session FROM target) AND type = 'assistant';

-- First step: parts live in session_part, the row keeps `content: []`.
INSERT INTO session_part (message_id, session_id, ordinal, type, call_id, time_created, time_updated, data)
  SELECT m.id, m.session_id, j.key, json_extract(j.value, '$.type'),
         CASE json_extract(j.value, '$.type') WHEN 'tool' THEN json_extract(j.value, '$.id') END,
         m.time_created, m.time_created, j.value
  FROM session_message m, json_each(m.data, '$.content') j
  WHERE m.id = (SELECT id FROM steps WHERE n = 1);
UPDATE session_message SET data = json_set(data, '$.content', json('[]')) WHERE id = (SELECT id FROM steps WHERE n = 1);

-- Second step: reasoning native data under its pre-v2.1 name `state`.
UPDATE session_message SET data = (
  SELECT json_set(data, '$.content', json_group_array(json(
    CASE WHEN json_extract(j.value, '$.type') = 'reasoning'
      THEN json_set(json_remove(j.value, '$.native'), '$.state', json_extract(j.value, '$.native'))
      ELSE j.value END)))
  FROM json_each(data, '$.content') j)
WHERE id = (SELECT id FROM steps WHERE n = 2);

-- The attached image moves to `blob` and the row names it by `media:<sha256>`.
INSERT INTO blob (hash, mime, bytes, width, height, data, time_created) VALUES (
  '6b7fa434f92a8b80aab02d9bf1a12e49ffcae424e4013a1c4f68b67e3d2bbcd0', 'image/png', 70, 1, 1,
  unhex('89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da6364f8cf500f00038601805a347d6b0000000049454e44ae426082'),
  1793787748350);
UPDATE session_message SET data = json_set(json_remove(data, '$.files[0].data'), '$.files[0].uri',
  'media:6b7fa434f92a8b80aab02d9bf1a12e49ffcae424e4013a1c4f68b67e3d2bbcd0')
WHERE session_id = (SELECT session FROM target) AND type = 'user' AND json_array_length(data, '$.files') > 0;
INSERT INTO blob_ref (session_id, message_id, hash, seq)
  SELECT session_id, id, '6b7fa434f92a8b80aab02d9bf1a12e49ffcae424e4013a1c4f68b67e3d2bbcd0', seq FROM session_message
  WHERE session_id = (SELECT session FROM target) AND type = 'user' AND data LIKE '%"uri":"media:%';

-- Last step: the process died while it streamed.
UPDATE session_message SET data = json_remove(data, '$.time.completed', '$.content[0].time.completed')
WHERE id = (SELECT id FROM steps WHERE n = total);

-- A compaction that never finished.
INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data)
  SELECT 'msg_fixturecompactionrunning', session, 'compaction', 1000, 1793787749000, 1793787749000,
         '{"time":{"created":1793787749000},"status":"running","reason":"auto","summary":""}'
  FROM target;
UPDATE event_sequence SET seq = 1000 WHERE aggregate_id = (SELECT session FROM target);
