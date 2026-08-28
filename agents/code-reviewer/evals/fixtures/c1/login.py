def find_user(db, username):
    query = "SELECT * FROM users WHERE name = '%s'" % username
    return db.execute(query).fetchall()
