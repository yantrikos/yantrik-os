#!/usr/bin/env python3
"""Merge LoRA adapter into base model and export GGUF."""

import os
os.environ["TORCHDYNAMO_DISABLE"] = "1"

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
LORA_DIR = os.path.join(SCRIPT_DIR, "checkpoints", "final_lora")
MERGE_DIR = os.path.join(SCRIPT_DIR, "model_merged")
GGUF_DIR = os.path.join(SCRIPT_DIR, "model_gguf")

def main():
    from unsloth import FastLanguageModel

    print("Loading base model + LoRA adapter...")
    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=LORA_DIR,
        max_seq_length=1024,
        load_in_4bit=False,
        load_in_16bit=True,
    )

    # Extract inner tokenizer if needed (Qwen3.5 VL processor)
    if hasattr(tokenizer, 'tokenizer'):
        print(f"Extracting inner tokenizer from {type(tokenizer).__name__}")
        text_tokenizer = tokenizer.tokenizer
        if hasattr(tokenizer, 'chat_template') and not hasattr(text_tokenizer, 'chat_template'):
            text_tokenizer.chat_template = tokenizer.chat_template
        tokenizer = text_tokenizer

    # Merge LoRA into base
    os.makedirs(MERGE_DIR, exist_ok=True)
    print(f"Merging LoRA into base model -> {MERGE_DIR}")
    model.save_pretrained_merged(MERGE_DIR, tokenizer, save_method="merged_16bit")
    print("Merge complete!")

    # Export GGUF
    os.makedirs(GGUF_DIR, exist_ok=True)
    print(f"Exporting GGUF (q4_k_m) -> {GGUF_DIR}")
    model.save_pretrained_gguf(GGUF_DIR, tokenizer, quantization_method="q4_k_m")
    print("GGUF export complete!")

    # Show output files
    for d in [MERGE_DIR, GGUF_DIR]:
        print(f"\n{d}:")
        for f in os.listdir(d):
            size = os.path.getsize(os.path.join(d, f))
            print(f"  {f} ({size/1024/1024:.1f} MB)")

    print("\nDone! GGUF ready for deployment.")

if __name__ == "__main__":
    main()
