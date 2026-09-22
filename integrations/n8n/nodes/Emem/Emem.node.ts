import type {
	IDataObject,
	IExecuteFunctions,
	INodeExecutionData,
	INodeType,
	INodeTypeDescription,
} from 'n8n-workflow';
import { NodeOperationError } from 'n8n-workflow';

const EMEM_MCP_URL = 'https://emem.dev/mcp/full';

export class Emem implements INodeType {
	description: INodeTypeDescription = {
		displayName: 'emem',
		name: 'emem',
		icon: 'file:emem.svg',
		group: ['transform'],
		version: 1,
		subtitle: '={{$parameter["toolName"]}}',
		description:
			'Shared memory for AI agents. One address per fact, one signature you check. No key to read.',
		defaults: {
			name: 'emem',
		},
		inputs: ['main'],
		outputs: ['main'],
		properties: [
			{
				displayName: 'Tool',
				name: 'toolName',
				type: 'options',
				options: [
					{ name: 'emem_locate', value: 'emem_locate' },
					{ name: 'emem_recall', value: 'emem_recall' },
					{ name: 'emem_ask', value: 'emem_ask' },
					{ name: 'emem_verify_receipt', value: 'emem_verify_receipt' },
					{ name: 'emem_memory_token', value: 'emem_memory_token' },
					{ name: 'emem_entity', value: 'emem_entity' },
					{ name: 'emem_guard_verdict', value: 'emem_guard_verdict' },
					{ name: 'emem_find_similar', value: 'emem_find_similar' },
					{ name: 'emem_memory_contradictions', value: 'emem_memory_contradictions' },
					{ name: 'Custom Tool', value: 'custom' },
				],
				default: 'emem_locate',
				description:
					'The emem MCP tool to call. Choose "Custom Tool" to type any of the 108 tool names.',
			},
			{
				displayName: 'Custom Tool Name',
				name: 'customToolName',
				type: 'string',
				default: '',
				displayOptions: {
					show: { toolName: ['custom'] },
				},
				description:
					'The exact emem tool name (e.g. emem_ndvi, emem_soil). Full list at emem.dev/mcp.',
			},
			{
				displayName: 'Parameters (JSON)',
				name: 'parameters',
				type: 'json',
				default: '{}',
				description:
					'JSON object of parameters to pass to the tool. Schemas at emem.dev/mcp.',
			},
		],
	};

	async execute(this: IExecuteFunctions): Promise<INodeExecutionData[][]> {
		const items = this.getInputData();
		const returnData: INodeExecutionData[] = [];

		for (let i = 0; i < items.length; i++) {
			try {
				const toolName = this.getNodeParameter('toolName', i) as string;
				const actualTool =
					toolName === 'custom'
						? (this.getNodeParameter('customToolName', i) as string)
						: toolName;
				const rawParams = this.getNodeParameter('parameters', i) as string;
				const params = typeof rawParams === 'string' ? JSON.parse(rawParams) : rawParams;

				const body = {
					jsonrpc: '2.0',
					id: 1,
					method: 'tools/call',
					params: { name: actualTool, arguments: params },
				};

				const response = await this.helpers.httpRequest({
					method: 'POST',
					url: EMEM_MCP_URL,
					headers: { 'Content-Type': 'application/json' },
					body,
					json: true,
				});

				const content = response?.result?.content;
				let parsed: IDataObject = response as IDataObject;
				if (Array.isArray(content) && content.length > 0 && content[0].text) {
					try {
						parsed = JSON.parse(content[0].text) as IDataObject;
					} catch {
						parsed = { raw_text: content[0].text } as IDataObject;
					}
				}

				returnData.push({ json: parsed });
			} catch (error) {
				if (this.continueOnFail()) {
					returnData.push({ json: { error: (error as Error).message } });
				} else {
					throw new NodeOperationError(this.getNode(), error as Error, {
						itemIndex: i,
					});
				}
			}
		}

		return [returnData];
	}
}
