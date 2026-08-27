def load_customers(database):
    customers = database.fetch_all("select id, name from customers")
    return [customer for customer in customers]
