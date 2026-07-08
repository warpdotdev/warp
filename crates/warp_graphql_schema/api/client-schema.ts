import fetch from 'cross-fetch';
import { getIntrospectionQuery, buildClientSchema, GraphQLSchema } from 'graphql';
import { filterSchema, pruneSchema } from '@graphql-tools/utils';

const clientMutations = [
  'addInviteLinkDomainRestriction',
  'addObjectGuests',
  'bulkCreateObjects',
  'confirmFileArtifactUpload',
  'createAgentTask',
  'createAnonymousUser',
  'createFileArtifactUploadTarget',
  'createFolder',
  'createGenericStringObject',
  'createManagedMcpClientConfig',
  'createManagedSecret',
  'createNotebook',
  'createTeam',
  'createWorkflow',
  'deleteConversation',
  'deleteInviteLinkDomainRestriction',
  'deleteManagedSecret',
  'updateManagedSecret',
  'deleteObject',
  'deleteTeamInvite',
  'emptyTrash',
  'expireApiKey',
  'generateApiKey',
  'generateCodeEmbeddings',
  'generateCommands',
  'generateDialogue',
  'generateMetadataForCommand',
  'generateUniqueUpgradePromoCode',
  'giveUpNotebookEditAccess',
  'grabNotebookEditAccess',
  'issueTaskIdentityToken',
  'joinTeamWithTeamDiscovery',
  'leaveObject',
  'markAcceptedIntelligentAutosuggestion',
  'mintCustomToken',
  'moveObject',
  'populateMerkleTreeCache',
  'provideNegativeFeedbackResponseForAiConversation',
  'purchaseAddonCredits',
  'recordObjectAction',
  'removeObjectGuest',
  'removeObjectLinkPermissions',
  'removeUserFromTeam',
  'renameTeam',
  'resetInviteLinks',
  'sendOnboardingSurveyResponses',
  'sendReferralInviteEmails',
  'setObjectLinkPermissions',
  'sendTeamInviteEmail',
  'setIsInviteLinkEnabled',
  'setTeamDiscoverability',
  'setTeamMemberRole',
  'setUserIsOnboarded',
  'shareBlock',
  'stripeBillingPortal',
  'transferGenericStringObjectOwner',
  'transferNotebookOwner',
  'transferTeamOwnership',
  'transferWorkflowOwner',
  'trashObject',
  'unshareBlock',
  'untrashObject',
  'updateAgentTask',
  'updateFolder',
  'updateGenericStringObject',
  'updateNotebook',
  'updateMerkleTree',
  'updateObjectGuests',
  'updateUserSettings',
  'updateWorkflow',
  'updateWorkspaceSettings',
  'updateOnboardingSurveyStatus',
  'createSimpleIntegration',
];

const clientQueries = [
  'cloudObject',
  'codebaseContextConfig',
  'freeAvailableModels',
  'getRelevantFragments',
  'rerankFragments',
  'harnessAuthSecrets',
  'listWarpDevImages',
  'pricingInfo',
  'managedSecrets',
  'syncMerkleTree',
  'updatedCloudObjects',
  'user',
  'userGithubInfo',
  'userRepoAuthStatus',
  'apiKeys',
  'getOAuthConnectTxStatus',
  'getCloudEnvironments',
  'simpleIntegrations',
  'getIntegrationsUsingEnvironment',
  'scheduledAgentHistory',
  'task',
  'taskGitCredentials',
  'taskSecrets',
  'listAIConversations',
  'suggestCloudEnvironmentImage'
];

const clientSubscriptions = ['warpDriveUpdates'];

function filterToClient(schema: GraphQLSchema): GraphQLSchema {
  const filtered = filterSchema({
    schema,
    rootFieldFilter: (operation, rootFieldName) => {
      if (operation === 'Query') {
        return clientQueries.includes(rootFieldName);
      } else if (operation === 'Mutation') {
        return clientMutations.includes(rootFieldName);
      } else if (operation === 'Subscription') {
        return clientSubscriptions.includes(rootFieldName);
      } else {
        console.error(`Unknown operation ${operation}.${rootFieldName}`);
        return true;
      }
    }
  });

  return pruneSchema(filtered);
}

module.exports = async (schemaUrl: string) => {
  // Adapted from https://the-guild.dev/graphql/codegen/docs/config-reference/schema-field#custom-schema-loader
  const response = await fetch(schemaUrl, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ query: getIntrospectionQuery() }),
  });
  const data = await response.json();
  const schema = buildClientSchema(data.data);
  return filterToClient(schema);
};
