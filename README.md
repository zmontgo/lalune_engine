# La Lune Engine

## Design Philosophy

This program is designed to work with the website [La Lune](https://lalune.zachmontgomery.com/), separating the logic from the presentation, separating concerns by enforcing interaction over Redis as a message queue. The engine is responsible for handling the logic, caching, and API interactions, while the website is responsible for displaying the data and interacting with the user.

## Database Schema

The one exception to the nearly complete separation of concerns is the database schema. Since passing sensitive information over Redis is not secure, the website is responsible for storing the user's API key and secret. The engine will gracefully handle the case where the user has not yet provided their API key and secret, but will not be able to perform any actions that require authentication.

As the website also uses the database to store user authentication information and user challenges, it is most sensible to have the website manage the schema as the source of truth. SQLx will ensure compile-time safety of the queries.

**If you are experiencing issues with the database schema, it is most likely due to not having run the Prisma migrations on the website side.**