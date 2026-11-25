import { Plugin } from "@opencode/plugin"

const toolName = "factory_save_intent"
const approval = "factory_direct_intent"

export default Plugin.define({
  id: "yoloforge.direct-intents",
  async setup(ctx) {
    const isDirect = async (sessionID) => {
      const session = await ctx.session.get({ sessionID })
      return session.metadata?.factory_direct === true && !session.parentID
    }
    await ctx.tool.transform((editor) => {
      editor.add({
        name: toolName,
        description: "Save an Intent in this Direct's YoloForge project. Use enqueue=false for a draft. With enqueue=true the user reviews and confirms the exact Intent in the Direct tab before Factory enqueues it.",
        options: { codemode: false },
        input: {
          type: "object",
          properties: {
            title: { type: "string", minLength: 1, maxLength: 80 },
            prompt: { type: "string", minLength: 1, maxLength: 12000, description: "Markdown implementation brief, including scope and acceptance criteria." },
            enqueue: { type: "boolean" },
          },
          required: ["title", "prompt", "enqueue"],
          additionalProperties: false,
        },
        execute: async (input, context) => {
          if (!await isDirect(context.sessionID)) throw new Error("Intent creation is only available in a Direct")
          const title = input.title.trim()
          const prompt = input.prompt.trim()
          if (!title || /[\u0000-\u001f\u007f]/u.test(title) || !prompt ||
            /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(prompt) ||
            new TextEncoder().encode(prompt).length > 24 * 1024) throw new Error("Invalid Intent title or brief")
          await ctx.permission.assert({
            sessionID: context.sessionID,
            agent: context.agent,
            source: { type: "tool", messageID: context.messageID, id: context.id },
            action: approval,
            resources: [context.id],
            save: [],
            metadata: { direct_intent: { title, prompt, enqueue: input.enqueue } },
          })
          return { content: "The Intent request was acknowledged. The Factory Intent list shows the actual saved or queued result." }
        },
      })
    })
    await ctx.session.hook("context", async (event) => {
      if (!event.sessionID || !await isDirect(event.sessionID)) delete event.tools[toolName]
      else delete event.tools.question
    })
  },
})
