FROM pytorch/pytorch:2.6.0-cuda12.6-cudnn9-devel

ENV DEBIAN_FRONTEND=noninteractive
ENV TORCHDYNAMO_DISABLE=1
ENV PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

# Install training dependencies
RUN pip install --no-cache-dir \
    "unsloth[cu126-torch260]" \
    accelerate \
    datasets \
    trl \
    peft \
    bitsandbytes

WORKDIR /training
