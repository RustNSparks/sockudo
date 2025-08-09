# DynamoDB Migration for allowed_origins

DynamoDB is schema-less, so no migration script is needed. The `allowed_origins` field will be added automatically when you update an application with origin restrictions.

## Field Format

The `allowed_origins` field should be stored as a List of Strings (L attribute) in DynamoDB:

```json
{
  "id": { "S": "your-app-id" },
  "key": { "S": "your-app-key" },
  "secret": { "S": "your-app-secret" },
  "allowed_origins": {
    "L": [
      { "S": "https://app.example.com" },
      { "S": "*.staging.example.com" },
      { "S": "http://localhost:3000" }
    ]
  }
}
```

## Using AWS CLI to Update

```bash
aws dynamodb update-item \
    --table-name sockudo-applications \
    --key '{"id": {"S": "your-app-id"}}' \
    --update-expression "SET allowed_origins = :origins" \
    --expression-attribute-values '{
        ":origins": {
            "L": [
                {"S": "https://app.example.com"},
                {"S": "*.staging.example.com"},
                {"S": "http://localhost:3000"}
            ]
        }
    }'
```

## Using AWS Console

1. Open DynamoDB Console
2. Select your applications table
3. Find the application item you want to update
4. Add or edit the `allowed_origins` attribute
5. Set the attribute type to List
6. Add String values for each allowed origin