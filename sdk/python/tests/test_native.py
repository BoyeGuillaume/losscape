import losscape


def test_version_is_exposed():
    assert losscape.__version__
    assert losscape.PROTOCOL_VERSION == 0


def test_run_ids_are_unique_and_sortable():
    ids = [losscape.new_run_id() for _ in range(100)]
    assert len(set(ids)) == 100
    assert all(len(i) == 32 for i in ids)
    assert ids == sorted(ids)
