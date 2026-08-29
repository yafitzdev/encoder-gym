use std::collections::BTreeMap;

use candle_core::{Tensor, Var, backprop::GradStore};

struct AdamVariable {
    parameter: Var,
    first_moment: Var,
    second_moment: Var,
}

pub(crate) struct SerializableAdamW {
    variables: BTreeMap<String, AdamVariable>,
    step: u64,
    beta1: f64,
    beta2: f64,
    epsilon: f64,
    weight_decay: f64,
}

impl SerializableAdamW {
    pub fn new(variables: BTreeMap<String, Var>, weight_decay: f64) -> candle_core::Result<Self> {
        let variables = variables
            .into_iter()
            .filter(|(_, variable)| variable.dtype().is_float())
            .map(|(name, parameter)| {
                let first_moment =
                    Var::zeros(parameter.shape(), parameter.dtype(), parameter.device())?;
                let second_moment =
                    Var::zeros(parameter.shape(), parameter.dtype(), parameter.device())?;
                Ok((
                    name,
                    AdamVariable {
                        parameter,
                        first_moment,
                        second_moment,
                    },
                ))
            })
            .collect::<candle_core::Result<BTreeMap<_, _>>>()?;
        Ok(Self {
            variables,
            step: 0,
            beta1: 0.9,
            beta2: 0.999,
            epsilon: 1e-8,
            weight_decay,
        })
    }

    pub fn step(
        &mut self,
        gradients: &GradStore,
        learning_rate: f64,
        clip_norm: f64,
    ) -> candle_core::Result<()> {
        self.step += 1;
        let gradient_norm = self.gradient_norm(gradients)?;
        let gradient_scale = if gradient_norm > clip_norm {
            clip_norm / gradient_norm
        } else {
            1.0
        };
        let first_scale = 1.0 / (1.0 - self.beta1.powi(self.step as i32));
        let second_scale = 1.0 / (1.0 - self.beta2.powi(self.step as i32));
        for variable in self.variables.values() {
            let Some(gradient) = gradients.get(&variable.parameter) else {
                continue;
            };
            let gradient = (gradient * gradient_scale)?;
            let next_first = ((variable.first_moment.as_tensor() * self.beta1)?
                + (&gradient * (1.0 - self.beta1))?)?;
            let next_second = ((variable.second_moment.as_tensor() * self.beta2)?
                + (gradient.sqr()? * (1.0 - self.beta2))?)?;
            let corrected_first = (&next_first * first_scale)?;
            let corrected_second = (&next_second * second_scale)?;
            let decayed =
                (variable.parameter.as_tensor() * (1.0 - learning_rate * self.weight_decay))?;
            let update = (corrected_first / (corrected_second.sqrt()? + self.epsilon)?)?;
            variable
                .parameter
                .set(&(decayed - (update * learning_rate)?)?)?;
            variable.first_moment.set(&next_first)?;
            variable.second_moment.set(&next_second)?;
        }
        Ok(())
    }

    fn gradient_norm(&self, gradients: &GradStore) -> candle_core::Result<f64> {
        let mut squared_norm = 0.0;
        for variable in self.variables.values() {
            if let Some(gradient) = gradients.get(&variable.parameter) {
                squared_norm += gradient.sqr()?.sum_all()?.to_scalar::<f32>()? as f64;
            }
        }
        Ok(squared_norm.sqrt())
    }

    pub fn step_count(&self) -> u64 {
        self.step
    }

    pub fn state_tensors(&self) -> BTreeMap<String, Tensor> {
        let mut tensors = BTreeMap::new();
        for (name, variable) in &self.variables {
            tensors.insert(
                format!("optimizer.first.{name}"),
                variable.first_moment.as_tensor().clone(),
            );
            tensors.insert(
                format!("optimizer.second.{name}"),
                variable.second_moment.as_tensor().clone(),
            );
        }
        tensors
    }

    pub fn restore(
        &mut self,
        step: u64,
        tensors: &BTreeMap<String, Tensor>,
    ) -> candle_core::Result<()> {
        for (name, variable) in &self.variables {
            let first = tensors
                .get(&format!("optimizer.first.{name}"))
                .ok_or_else(|| {
                    candle_core::Error::Msg(format!("missing first moment for {name}"))
                })?;
            let second = tensors
                .get(&format!("optimizer.second.{name}"))
                .ok_or_else(|| {
                    candle_core::Error::Msg(format!("missing second moment for {name}"))
                })?;
            variable.first_moment.set(first)?;
            variable.second_moment.set(second)?;
        }
        self.step = step;
        Ok(())
    }
}
