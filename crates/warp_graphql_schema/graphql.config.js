module.exports = {
  projects: {
    // Download the GraphQL schema from the staging server.
    staging: {
      schema: {
        'https://staging.warp.dev/graphql/v2': {
          loader: './api/client-schema.ts'
        }
      },
      extensions: {
        codegen: {
          generates: {
            'api/schema.graphql': {
              plugins: ['schema-ast']
            }
          }
        }
      }
    },
    // Download the GraphQL schema from a local server.
    local: {
      schema: {
        'http://localhost:8080/graphql/v2': {
          loader: './api/client-schema.ts'
        }
      },
      extensions: {
        codegen: {
          generates: {
            'api/schema.graphql': {
              plugins: ['schema-ast']
            }
          }
        }
      }
    },
  }
};
