# IronCalc AppServer

This is the Application server deployed at https://app.ironcalc.com

It is a simple Rocket server. It is assumed to run alongside a file-server

All /api/ RPCs will go to this server

## Documents API

Workbooks kept on the server, for one owner. Every `/api/documents` request needs `Authorization: Bearer <token>`.

The token comes from the `IRONCALC_API_TOKEN` environment variable. Without it the server reads the file named by `IRONCALC_API_TOKEN_FILE`, or `.api-token` in its working directory, and creates that file with a random token the first time.

| Request | What it does |
| --- | --- |
| `GET /api/documents` | Lists the documents |
| `POST /api/documents?name=` | Stores a new workbook sent as IronCalc bytes |
| `GET /api/documents/<id>` | Returns the bytes, with the version in `X-Document-Version` |
| `PUT /api/documents/<id>?version=&name=` | Replaces the bytes if `version` is still current, otherwise answers 409 with `current_version` |
| `PATCH /api/documents/<id>` | Renames, from `{"name": "..."}` |
| `DELETE /api/documents/<id>` | Deletes |
| `GET /api/documents/<id>/values?sheet=&range=` | Returns calculated values of a range, row by row |
