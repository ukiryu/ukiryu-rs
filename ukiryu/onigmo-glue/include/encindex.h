/*
 * Ruby's encoding index table. Onigmo reads the index only to tell dummy
 * encodings apart (ENC_DUMMY_FLAG); the encodings built here are not dummy,
 * so each source file's ENCINDEX_* fallback of 0 applies.
 */
